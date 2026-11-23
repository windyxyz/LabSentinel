//! 推理后端（T-325）：YOLO11n 人体检测，双后端可切换。
//!
//! - `tract`（feature=tract）：纯 Rust ONNX 运行时，无 C++/动态库依赖——
//!   windows-gnu 直接构建，"无 ORT 环境"的兜底实现；
//! - `ort`（feature=ort，T-325 原定方案/T-328 验收口径）：`load-dynamic` 运行时加载
//!   `ORT_DYLIB_PATH` 指定的 onnxruntime.dll——与 Python 同后端，黄金语料事件一致率
//!   ≥98% 的验收在此口径下进行。rc.10+ 绑定层纯 Rust（无 bindgen/libclang）。
//!
//! 预处理与 Python `engine.py` 对齐：letterbox 114 灰边 + 双线性缩放 + BGR→RGB +
//! /255 + NCHW；解码/NMS 复用 [`crate::post`]（与 Python 逐位对齐的既有实现）。
//! 后端由 [`YoloModel::load`] 的 `backend` 参数或环境变量 `LS_INFER_BACKEND`
//! （"tract"|"ort"）选择。

use std::path::Path;
use std::time::Duration;

#[cfg(feature = "ort")]
use ort::session::Session;
#[cfg(feature = "ort")]
use ort::value::{TensorElementType, ValueType};
#[cfg(feature = "tract")]
use tract_onnx::prelude::*;

use crate::post::{self, Det, LetterboxGeom};

#[derive(Clone, Copy, Default)]
pub struct InferenceTimings {
    pub run: Duration,
    pub directml_wait: Duration,
    pub calls: u64,
    pub directml_calls: u64,
}

thread_local! {
    static INFERENCE_TIMINGS: std::cell::Cell<InferenceTimings> = const {
        std::cell::Cell::new(InferenceTimings { run: Duration::ZERO, directml_wait: Duration::ZERO, calls: 0, directml_calls: 0 })
    };
}

/// Collect model calls from the current worker thread since its previous frame.
pub fn take_inference_timings() -> InferenceTimings {
    INFERENCE_TIMINGS.with(|metrics| metrics.replace(InferenceTimings::default()))
}

/// 推理后端种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Tract,
    Ort,
}

impl Backend {
    /// 从环境变量解析（缺省 tract；仅当对应 feature 启用且可用时返回 Ort）。
    pub fn from_env() -> Option<Backend> {
        match std::env::var("LS_INFER_BACKEND")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "ort" => {
                #[cfg(feature = "ort")]
                {
                    Some(Backend::Ort)
                }
                #[cfg(not(feature = "ort"))]
                {
                    eprintln!("[ls-infer] LS_INFER_BACKEND=ort 但 ort feature 未启用——回退 tract");
                    #[cfg(feature = "tract")]
                    {
                        Some(Backend::Tract)
                    }
                    #[cfg(not(feature = "tract"))]
                    {
                        None
                    }
                }
            }
            "tract" => {
                #[cfg(feature = "tract")]
                {
                    Some(Backend::Tract)
                }
                #[cfg(not(feature = "tract"))]
                {
                    None
                }
            }
            _ => {
                // UX（2026-11-21）：缺省 **ort**（GPU/DirectML 路线的前提——tract
                // 纯 Rust 推理无法走 GPU；发行形态自带 DML 版 onnxruntime.dll）。
                // tract 仍可显式 `LS_INFER_BACKEND=tract`（黄金语料对齐用）。
                #[cfg(feature = "ort")]
                {
                    Some(Backend::Ort)
                }
                #[cfg(not(feature = "ort"))]
                {
                    #[cfg(feature = "tract")]
                    {
                        Some(Backend::Tract) // 无 ort feature 时回退 tract
                    }
                    #[cfg(not(feature = "tract"))]
                    {
                        None
                    }
                }
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Tract => "tract",
            Backend::Ort => "ort",
        }
    }
}

/// T-334/Q-14：读取模型同目录的 `model.json` 角色声明（roles 对象）。
/// 文件缺失/字段非法一律回退空对象（旧模型行为不变，绝不阻断推理）。
fn read_model_roles(model_path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let Some(dir) = model_path.parent() else {
        return serde_json::Map::new();
    };
    let Ok(text) = std::fs::read_to_string(dir.join("model.json")) else {
        return serde_json::Map::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return serde_json::Map::new();
    };
    v.get("roles")
        .and_then(|r| r.as_object())
        .cloned()
        .unwrap_or_default()
}

/// LS-055：模型目录 model.json 可声明输入尺寸（`imgsz` / `input_size` /
/// `input: [w,h]`），用于 960/1280 等非默认尺寸的模型；缺省回退调用方传入值。
fn read_model_imgsz(model_path: &Path) -> Option<u32> {
    let dir = model_path.parent()?;
    let text = std::fs::read_to_string(dir.join("model.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if let Some(n) = v.get("imgsz").and_then(|x| x.as_u64()) {
        if (64..=4096).contains(&n) {
            return Some(n as u32);
        }
    }
    if let Some(n) = v.get("input_size").and_then(|x| x.as_u64()) {
        if (64..=4096).contains(&n) {
            return Some(n as u32);
        }
    }
    if let Some(arr) = v.get("input").and_then(|x| x.as_array()) {
        let w = arr.first().and_then(|x| x.as_u64())?;
        let h = arr.get(1).and_then(|x| x.as_u64())?;
        if w == h && (64..=4096).contains(&w) {
            return Some(w as u32);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Yolo,
    RtdetrScores,
    RtdetrTopk,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Option<Self>, String> {
        Ok(match value {
            "auto" => None,
            "yolo" => Some(Self::Yolo),
            "rtdetr_scores" => Some(Self::RtdetrScores),
            "rtdetr_topk" => Some(Self::RtdetrTopk),
            _ => return Err(format!("未知的模型输出格式: {value}")),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yolo => "yolo",
            Self::RtdetrScores => "rtdetr_scores",
            Self::RtdetrTopk => "rtdetr_topk",
        }
    }

    #[cfg(feature = "ort")]
    fn from_metadata(session: &Session) -> Result<Self, String> {
        let Ok(meta) = session.metadata() else {
            return Ok(Self::Yolo);
        };
        if let Some(format) = meta.custom("labsentinel_output_format") {
            if let Some(format) = Self::parse(&format)? {
                return Ok(format);
            }
        }
        if meta.custom("head").as_deref() == Some("RTDETRDecoder") {
            return Ok(Self::RtdetrTopk);
        }
        if meta.custom("description").is_some_and(|description| {
            let description = description.to_ascii_lowercase();
            ["ultralytics rtdetr-", "ultralytics rt-detr-"]
                .iter()
                .any(|prefix| description.starts_with(prefix))
        }) {
            return Ok(Self::RtdetrScores);
        }
        Ok(Self::Yolo)
    }
}

fn model_format_override(path: &Path) -> Result<Option<OutputFormat>, String> {
    let text = match std::fs::read_to_string(path.with_extension("model.json")) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("读取模型配置失败: {err}")),
    };
    let config: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("模型配置 JSON 无效: {e}"))?;
    config.get("output_format").map_or(Ok(None), |value| {
        OutputFormat::parse(value.as_str().ok_or("output_format 必须是字符串")?)
    })
}

/// Run a prepared RGB NCHW tensor and return the first model output.
pub trait InferBackend: Send {
    fn run(
        &mut self,
        data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String>;
}

#[cfg(feature = "tract")]
struct TractBackend {
    model: std::sync::Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>,
}

#[cfg(feature = "tract")]
impl InferBackend for TractBackend {
    fn run(
        &mut self,
        data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        let input = tract_ndarray::Array4::<f32>::from_shape_vec(
            (1, 3, height as usize, width as usize),
            data,
        )
        .map_err(|e| format!("输入张量形状错误: {e}"))?;
        let run_started = std::time::Instant::now();
        let outputs = self
            .model
            .run(tvec!(input.into_tvalue()))
            .map_err(|e| format!("推理失败: {e}"))?;
        INFERENCE_TIMINGS.with(|metrics| {
            let mut totals = metrics.get();
            totals.run += run_started.elapsed();
            totals.calls += 1;
            metrics.set(totals);
        });
        let view = outputs
            .first()
            .ok_or("模型无输出")?
            .to_plain_array_view::<f32>()
            .map_err(|e| format!("输出类型错误: {e}"))?;
        Ok((view.shape().to_vec(), view.iter().copied().collect()))
    }
}

#[cfg(feature = "ort")]
struct OrtBackend {
    session: Option<Session>,
    is_dml: bool,
    fp16: bool,
}

#[cfg(feature = "ort")]
impl Drop for OrtBackend {
    fn drop(&mut self) {
        if self.is_dml && self.session.is_some() {
            let _guard = dml_lock().lock().unwrap_or_else(|e| e.into_inner());
            self.session.take();
        }
    }
}

#[cfg(feature = "ort")]
impl InferBackend for OrtBackend {
    fn run(
        &mut self,
        data: Vec<f32>,
        width: u32,
        height: u32,
    ) -> Result<(Vec<usize>, Vec<f32>), String> {
        let session = self.session.as_mut().ok_or("ORT 会话未初始化")?;
        let dims = [1usize, 3, height as usize, width as usize];
        let tensor = if self.fp16 {
            let data = data
                .into_iter()
                .map(half::f16::from_f32)
                .collect::<Vec<_>>();
            ort::value::Tensor::from_array((dims, data)).map(|tensor| tensor.into_dyn())
        } else {
            ort::value::Tensor::from_array((dims, data)).map(|tensor| tensor.into_dyn())
        }
        .map_err(|e| format!("输入张量创建失败: {e}"))?;
        // 事故 A1 防护：DirectML 推理必须跨线程串行；CPU 推理不加锁。
        let waiting_since = std::time::Instant::now();
        let _dml_guard = if self.is_dml {
            Some(dml_lock().lock().unwrap_or_else(|e| e.into_inner()))
        } else {
            None
        };
        let run_started = std::time::Instant::now();
        let outputs = session.run(ort::inputs![tensor]);
        INFERENCE_TIMINGS.with(|metrics| {
            let mut totals = metrics.get();
            totals.run += run_started.elapsed();
            if self.is_dml {
                totals.directml_wait += run_started - waiting_since;
                totals.directml_calls += 1;
            }
            totals.calls += 1;
            metrics.set(totals);
        });
        let outputs = outputs.map_err(|e| format!("推理失败: {e}"))?;
        // YOLO 单输出：取首个输出（SessionOutputs 可迭代，取第一个键值对）
        let (_name, v) = outputs.iter().next().ok_or("模型无输出")?;
        let result = if let Ok((shape, flat)) = v.try_extract_tensor::<f32>() {
            (shape.iter().map(|&d| d as usize).collect(), flat.to_vec())
        } else {
            let (shape, flat) = v
                .try_extract_tensor::<half::f16>()
                .map_err(|e| format!("输出类型错误: {e}"))?;
            (
                shape.iter().map(|&d| d as usize).collect(),
                flat.iter().map(|v| v.to_f32()).collect(),
            )
        };
        drop(v);
        drop(outputs);
        drop(_dml_guard);
        Ok(result)
    }
}

/// YOLO11 ONNX 会话（后端无关，`person_class` 由 model.json 角色声明，缺省 0）。
pub struct YoloModel {
    backend: Box<dyn InferBackend>,
    output_format: OutputFormat,
    input_w: u32,
    input_h: u32,
    person_class: usize,
    conf: f32,
    iou: f32,
    min_box_area: f32,
}

/// 进程级 DirectML 串行锁（事故 A1 防护：AMD/DX12 驱动下 ORT DirectML
/// `Session::run` / 建会话 / 释放严禁跨线程并发，否则导致 Windows 整机冻结）。
/// CPU 与 tract 路径不加此锁。
#[cfg(feature = "ort")]
pub fn dml_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &LOCK
}

#[cfg(feature = "ort")]
fn ort_environment() -> Result<&'static ort::environment::Environment, String> {
    static ENVIRONMENT: std::sync::OnceLock<Result<ort::environment::Environment, String>> =
        std::sync::OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| {
            ort::environment::init()
                .with_name("labsentinel")
                .build()
                .map_err(|e| format!("ORT 环境初始化失败: {e}（ORT_DYLIB_PATH 是否设置？）"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(feature = "ort")]
pub(crate) fn load_ort_session(path: &Path, use_gpu: bool) -> Result<(Session, bool), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取模型失败: {e}"))?;
    let environment = ort_environment()?;
    let build = |with_dml| -> Result<Session, String> {
        let mut builder = Session::builder(environment).map_err(|e| e.to_string())?;
        if with_dml {
            // DirectML requires sequential execution and disables memory patterns.
            builder = builder
                .with_memory_pattern(false)
                .map_err(|e| e.to_string())?
                .with_parallel_execution(false)
                .map_err(|e| e.to_string())?
                .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
                .map_err(|e| e.to_string())?;
        }
        builder
            .commit_from_memory(&bytes)
            .map_err(|e| format!("ORT 加载模型失败: {e}"))
    };
    if use_gpu {
        let _guard = dml_lock().lock().unwrap_or_else(|e| e.into_inner());
        match build(true) {
            Ok(session) => return Ok((session, true)),
            Err(e) => eprintln!("[ls-infer] GPU(DirectML) 初始化失败，自动回落 CPU: {e}"),
        }
    }
    Ok((build(false)?, false))
}

/// 实际生效的推理设备（"DirectML(GPU)" / "CPU" / "CPU(tract)"）——每次 load
/// 成功后覆盖更新，网页切换 GPU/CPU 并软重启相机后 `/api/system` 实时反映最新设备。
static EP_USED: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

pub fn ep_used() -> Option<String> {
    EP_USED.read().ok().and_then(|g| g.clone())
}

pub fn set_ep_used(val: impl Into<String>) {
    if let Ok(mut g) = EP_USED.write() {
        *g = Some(val.into());
    }
}

impl YoloModel {
    pub fn use_output_format(&mut self, format: Option<OutputFormat>) {
        if let Some(format) = format {
            self.output_format = format;
        }
    }

    pub fn output_format(&self) -> OutputFormat {
        self.output_format
    }

    pub fn input_dimensions(&self) -> (u32, u32) {
        (self.input_w, self.input_h)
    }

    /// 加载模型。`backend=None` 时按 `LS_INFER_BACKEND` 解析（缺省 tract）。
    /// `use_gpu=true` 时注册 DirectML EP（DX12 统一层——N 卡/A 卡/Intel i 卡/
    /// 核显全通吃），**失败自动回落 CPU**（任意设备鲁棒；实际生效设备见日志）。
    pub fn load(
        path: &Path,
        imgsz: u32,
        conf: f32,
        iou: f32,
        min_box_area: f32,
        backend: Option<Backend>,
        use_gpu: bool,
    ) -> Result<Self, String> {
        let backend = backend
            .or_else(Backend::from_env)
            .ok_or("无可用推理后端（tract/ort feature 均未启用）")?;
        let fallback = read_model_imgsz(path).unwrap_or(imgsz);
        let format_override = model_format_override(path)?;
        let (backend, (input_w, input_h), output_format): (Box<dyn InferBackend>, _, _) =
            match backend {
                #[cfg(feature = "tract")]
                Backend::Tract => {
                    let mut graph = tract_onnx::onnx()
                        .model_for_path(path)
                        .map_err(|e| format!("加载模型失败: {e}"))?;
                    let fact = graph.input_fact(0).map_err(|e| e.to_string())?;
                    let dims = fact.shape.as_concrete_finite().map_err(|e| e.to_string())?;
                    let shape = if let Some(dims) = dims {
                        let dims: Vec<_> = dims.iter().map(|&d| d as i64).collect();
                        input_size(&dims, fallback)?
                    } else {
                        graph
                            .set_input_fact(
                                0,
                                InferenceFact::dt_shape(
                                    f32::datum_type(),
                                    tvec!(1, 3, fallback as usize, fallback as usize),
                                ),
                            )
                            .map_err(|e| e.to_string())?;
                        (fallback, fallback)
                    };
                    let m = graph
                        .into_optimized()
                        .map_err(|e| format!("模型优化失败: {e}"))?
                        .into_runnable()
                        .map_err(|e| format!("模型编译失败: {e}"))?;
                    set_ep_used("CPU(tract)");
                    (
                        Box::new(TractBackend { model: m }),
                        shape,
                        format_override.unwrap_or(OutputFormat::Yolo),
                    )
                }
                #[cfg(not(feature = "tract"))]
                Backend::Tract => return Err("tract feature 未启用".into()),
                #[cfg(feature = "ort")]
                Backend::Ort => {
                    let (session, is_dml) = load_ort_session(path, use_gpu)?;
                    let ep_used = if is_dml { "DirectML(GPU)" } else { "CPU" };
                    println!("[ls-infer] ORT 推理设备: {ep_used}");
                    let mut runtime = OrtBackend {
                        session: Some(session),
                        is_dml,
                        fp16: false,
                    };
                    let session = runtime.session.as_ref().ok_or("ORT 会话未初始化")?;
                    let output_format = match format_override {
                        Some(format) => format,
                        None => OutputFormat::from_metadata(session)?,
                    };
                    let input = session.inputs().first().ok_or("ORT 模型无输入")?;
                    let ValueType::Tensor { ty, shape, .. } = input.dtype() else {
                        return Err("ORT 模型输入必须是图像张量".into());
                    };
                    runtime.fp16 = match ty {
                        TensorElementType::Float16 => true,
                        TensorElementType::Float32 => false,
                        _ => return Err(format!("不支持的输入类型: {ty:?}")),
                    };
                    let shape = input_size(shape, fallback)?;
                    set_ep_used(ep_used);
                    (Box::new(runtime), shape, output_format)
                }
                #[cfg(not(feature = "ort"))]
                Backend::Ort => return Err("ort feature 未启用".into()),
            };
        Ok(YoloModel {
            backend,
            output_format,
            input_w,
            input_h,
            person_class: read_model_roles(path)
                .get("person_class")
                .and_then(|value| value.as_u64())
                .map_or(0, |value| value as usize),
            conf,
            iou,
            min_box_area,
        })
    }

    /// 对一帧 BGR raw 做检测，返回原图坐标 xyxy（已 NMS + 最小面积过滤）。
    pub fn detect(&mut self, bgr: &[u8], w: u32, h: u32) -> Result<Vec<Det>, String> {
        self.detect_class(bgr, w, h, self.person_class, self.conf, self.min_box_area)
    }

    /// Decode people and the requested context categories from one forward pass.
    pub fn detect_with_context(
        &mut self,
        bgr: &[u8],
        w: u32,
        h: u32,
        context: &[(usize, f32, f32)],
    ) -> Result<(Vec<Det>, Vec<Vec<Det>>), String> {
        let mut specs = vec![(self.person_class, self.conf, self.min_box_area)];
        specs.extend_from_slice(context);
        let mut groups = self.detect_classes(bgr, w, h, &specs)?.into_iter();
        let people = groups.next().unwrap_or_default();
        Ok((people, groups.collect()))
    }

    /// T-325/Q-14：按**类别索引 + 独立阈值**检测（单类便捷入口）。
    pub fn detect_class(
        &mut self,
        bgr: &[u8],
        w: u32,
        h: u32,
        class_id: usize,
        conf: f32,
        min_area: f32,
    ) -> Result<Vec<Det>, String> {
        let mut all = self.detect_classes(bgr, w, h, &[(class_id, conf, min_area)])?;
        Ok(all.pop().unwrap_or_default())
    }

    /// LS-043：单次前向推理多类解码（1 次 letterbox + 1 次 session.run 同时输出多个类别，
    /// 消除 PPE 级联每帧 2+ 次整图重复推理，提速 ~2×）。
    pub fn detect_classes(
        &mut self,
        bgr: &[u8],
        w: u32,
        h: u32,
        specs: &[(usize, f32, f32)],
    ) -> Result<Vec<Vec<Det>>, String> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        let (data, geom) = match self.output_format {
            OutputFormat::Yolo => {
                let (data, geom) = letterbox_nchw_wh(bgr, w, h, self.input_w, self.input_h)?;
                (data, BoxGeometry::Letterbox(geom))
            }
            _ => (
                stretch_nchw(bgr, w, h, self.input_w, self.input_h)?,
                BoxGeometry::Normalized { w, h },
            ),
        };
        let (shape, flat) = self.backend.run(data, self.input_w, self.input_h)?;
        let rows = OutputRows::new(self.output_format, &shape, &flat)?;
        Ok(specs
            .iter()
            .map(|&(cid, conf, min_area)| rows.decode(&geom, cid, conf, min_area, self.iou))
            .collect())
    }
}

fn input_size(shape: &[i64], fallback: u32) -> Result<(u32, u32), String> {
    let [batch, channels, height, width] = shape else {
        return Err(format!("模型输入必须为 NCHW，实得 {shape:?}"));
    };
    if *batch > 1 || (*channels > 0 && *channels != 3) {
        return Err(format!("模型输入必须支持单张 RGB 图像，实得 {shape:?}"));
    }
    let dimension = |n: i64| {
        let n = if n > 0 { n } else { i64::from(fallback) };
        (1..=4096)
            .contains(&n)
            .then_some(n as u32)
            .ok_or_else(|| format!("模型输入尺寸超出范围: {n}"))
    };
    Ok((dimension(*width)?, dimension(*height)?))
}

enum BoxGeometry {
    Letterbox(LetterboxGeom),
    Normalized { w: u32, h: u32 },
}

struct OutputRows<'a> {
    values: &'a [f32],
    channels: usize,
    anchors: usize,
    channel_major: bool,
    format: OutputFormat,
    yolo_nms_free: bool,
}

impl<'a> OutputRows<'a> {
    fn new(format: OutputFormat, shape: &[usize], values: &'a [f32]) -> Result<Self, String> {
        let [1, dim1, dim2] = *shape else {
            return Err(format!("输出维度异常: {shape:?}"));
        };
        let yolo_nms_free = format == OutputFormat::Yolo && dim2 == 6;
        let channel_major = format == OutputFormat::Yolo && !yolo_nms_free && dim1 <= dim2;
        let (channels, anchors) = if channel_major {
            (dim1, dim2)
        } else {
            (dim2, dim1)
        };
        if channels < 5
            || (format == OutputFormat::RtdetrTopk && channels != 6)
            || channels.checked_mul(anchors) != Some(values.len())
        {
            return Err(format!("输出维度与模型格式 {format:?} 不符: {shape:?}"));
        }
        Ok(Self {
            values,
            channels,
            anchors,
            channel_major,
            format,
            yolo_nms_free,
        })
    }

    fn get(&self, row: usize, col: usize) -> f32 {
        let index = if self.channel_major {
            col * self.anchors + row
        } else {
            row * self.channels + col
        };
        self.values[index]
    }

    fn class_score(&self, row: usize, class: usize) -> Option<f32> {
        if self.yolo_nms_free || self.format == OutputFormat::RtdetrTopk {
            let id = self.get(row, 5);
            return (id >= 0.0 && id.is_finite() && id.round() as usize == class)
                .then(|| self.get(row, 4));
        }
        let col = class.checked_add(4).filter(|&c| c < self.channels)?;
        let score = self.get(row, col);
        if self.format == OutputFormat::RtdetrScores {
            // Older Ultralytics RT-DETR assigns each query to its highest scoring class.
            let higher = (4..self.channels).any(|c| {
                let other = self.get(row, c);
                other > score || (other == score && c < col)
            });
            if higher {
                return None;
            }
        }
        Some(score)
    }

    fn decode(
        &self,
        geom: &BoxGeometry,
        class: usize,
        conf: f32,
        min_area: f32,
        iou: f32,
    ) -> Vec<Det> {
        let mut dets = Vec::new();
        for row in 0..self.anchors {
            let Some(score) = self
                .class_score(row, class)
                .filter(|s| s.is_finite() && *s > conf)
            else {
                continue;
            };
            let [a, b, c, d] = std::array::from_fn(|col| self.get(row, col));
            if [a, b, c, d].iter().any(|v| !v.is_finite()) {
                continue;
            }
            let box_ = match geom {
                BoxGeometry::Letterbox(g) if self.yolo_nms_free => [
                    (a - g.dx) / g.r,
                    (b - g.dy) / g.r,
                    (c - g.dx) / g.r,
                    (d - g.dy) / g.r,
                ],
                BoxGeometry::Letterbox(g) => post::decode_row(a, b, c, d, g),
                BoxGeometry::Normalized { w, h } => [
                    ((a - c / 2.0) * *w as f32).clamp(0.0, *w as f32),
                    ((b - d / 2.0) * *h as f32).clamp(0.0, *h as f32),
                    ((a + c / 2.0) * *w as f32).clamp(0.0, *w as f32),
                    ((b + d / 2.0) * *h as f32).clamp(0.0, *h as f32),
                ],
            };
            let width = box_[2] - box_[0];
            let height = box_[3] - box_[1];
            if box_.iter().all(|v| v.is_finite())
                && width > 0.0
                && height > 0.0
                && width * height >= min_area
            {
                dets.push(Det { box_, score });
            }
        }
        if self.format == OutputFormat::Yolo && !self.yolo_nms_free {
            return post::nms(dets, iou);
        }
        if self.format == OutputFormat::RtdetrScores {
            dets.sort_by(|a, b| b.score.total_cmp(&a.score));
        }
        dets
    }
}

/// letterbox + BGR→RGB + /255 + NCHW（与 Python `_letterbox` 同几何）。
/// 缩放复用 [`resize_u8_linear_cv2`]（cv2 INTER_LINEAR 定点逐位复刻）。
pub fn letterbox_nchw(
    bgr: &[u8],
    w: u32,
    h: u32,
    s: u32,
) -> Result<(Vec<f32>, LetterboxGeom), String> {
    letterbox_nchw_wh(bgr, w, h, s, s)
}

fn letterbox_nchw_wh(
    bgr: &[u8],
    w: u32,
    h: u32,
    in_w: u32,
    in_h: u32,
) -> Result<(Vec<f32>, LetterboxGeom), String> {
    validate_image(bgr, w, h, in_w, in_h)?;
    let g = post::letterbox_geom_wh(w, h, in_w, in_h);
    let nw = ((w as f32 * g.r).floor() as u32).max(1).min(in_w);
    let nh = ((h as f32 * g.r).floor() as u32).max(1).min(in_h);
    let resized = resize_u8_linear_cv2(bgr, w, h, nw, nh, 3);
    let data = rgb_nchw(
        &resized,
        (nw, nh),
        (in_w, in_h),
        (g.dx as usize, g.dy as usize),
    );
    Ok((data, g))
}

fn stretch_nchw(bgr: &[u8], w: u32, h: u32, in_w: u32, in_h: u32) -> Result<Vec<f32>, String> {
    validate_image(bgr, w, h, in_w, in_h)?;
    let resized = resize_u8_linear_cv2(bgr, w, h, in_w, in_h, 3);
    Ok(rgb_nchw(&resized, (in_w, in_h), (in_w, in_h), (0, 0)))
}

fn validate_image(bgr: &[u8], w: u32, h: u32, in_w: u32, in_h: u32) -> Result<(), String> {
    let expect = (w as usize) * (h as usize) * 3;
    if bgr.len() < expect || w == 0 || h == 0 || in_w == 0 || in_h == 0 {
        return Err(format!(
            "帧尺寸不符: len={} expect={expect} w={w} h={h}",
            bgr.len()
        ));
    }
    Ok(())
}

fn rgb_nchw(
    resized: &[u8],
    (nw, nh): (u32, u32),
    (in_w, in_h): (u32, u32),
    (dx, dy): (usize, usize),
) -> Vec<f32> {
    let pad = 114.0f32 / 255.0;
    let plane = (in_w as usize) * (in_h as usize);
    let mut data = vec![pad; plane * 3];
    // cv2.resize(INTER_LINEAR) 定点复刻：Python 先 cv2.resize(uint8) 再 /255，
    // 这里同样先产 uint8 缩放帧，保证逐位一致（±1/255 的浮点差会传播到 conf）。
    let (red, green_blue) = data.split_at_mut(plane);
    let (green, blue) = green_blue.split_at_mut(plane);
    for (y, line) in resized
        .chunks_exact(nw as usize * 3)
        .take(nh as usize)
        .enumerate()
    {
        let start = (y + dy) * in_w as usize + dx;
        let line_out = start..start + nw as usize;
        let (red, green, blue) = (
            &mut red[line_out.clone()],
            &mut green[line_out.clone()],
            &mut blue[line_out],
        );
        for (x, &[b, g, r]) in line.as_chunks::<3>().0.iter().enumerate() {
            red[x] = f32::from(r) / 255.0;
            green[x] = f32::from(g) / 255.0;
            blue[x] = f32::from(b) / 255.0;
        }
    }
    data
}

/// cv2 `resize(..., INTER_LINEAR)` 对 8U 输入的定点复刻（逐位一致）。
///
/// cv2 流程（modules/imgproc/src/resize.cpp）：源坐标 `f = (dst+0.5)*scale - 0.5`，
/// 整数部分截取（负坐标截 0、越上界截 src_len-2），小数部分量化为
/// `rint(f * 2048)`（INTER_RESIZE_COEF_BITS=11），水平/垂直各一对系数，
/// 像素加权和 `Σ src*coef + 2^21 >> 22`（系数总积 = 2048² = 2²²）。
/// `rint` 为 cvRound 的 round-half-to-even。输出 uint8（经 saturate）。
pub fn resize_u8_linear_cv2(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32, cn: usize) -> Vec<u8> {
    let len = dw as usize * dh as usize * cn;
    if sw == dw && sh == dh {
        return src[..len].to_vec();
    }
    const COEF_BITS: i32 = 11;
    const COEF_SCALE: i32 = 1 << COEF_BITS; // 2048
    const SHIFT: i32 = COEF_BITS * 2; // 22
    let rint = |v: f32| -> i32 { (v.round_ties_even()) as i32 };
    let hcoef = |dx: u32| -> (usize, i32, i32) {
        let scale = sw as f64 / dw as f64;
        let f = ((dx as f64 + 0.5) * scale - 0.5) as f32;
        let mut ix = f.floor();
        let mut fx = f - ix;
        if ix < 0.0 {
            fx = 0.0;
            ix = 0.0;
        }
        if ix >= (sw as f32 - 1.0) {
            fx = 0.0;
            ix = sw as f32 - 2.0;
        }
        let c1 = rint(fx * COEF_SCALE as f32).clamp(0, COEF_SCALE);
        (ix.max(0.0) as usize, COEF_SCALE - c1, c1)
    };
    let vcoef = |dy: u32| -> (usize, i32, i32) {
        let scale = sh as f64 / dh as f64;
        let f = ((dy as f64 + 0.5) * scale - 0.5) as f32;
        let mut iy = f.floor();
        let mut fy = f - iy;
        if iy < 0.0 {
            fy = 0.0;
            iy = 0.0;
        }
        if iy >= (sh as f32 - 1.0) {
            fy = 0.0;
            iy = sh as f32 - 2.0;
        }
        let c1 = rint(fy * COEF_SCALE as f32).clamp(0, COEF_SCALE);
        (iy.max(0.0) as usize, COEF_SCALE - c1, c1)
    };
    let horizontal: Vec<_> = (0..dw).map(hcoef).collect();
    let mut dst = vec![0u8; len];
    for dy in 0..dh {
        let (y0, cy0, cy1) = vcoef(dy);
        let y1 = (y0 + 1).min(sh as usize - 1);
        for (dx, &(x0, cx0, cx1)) in horizontal.iter().enumerate() {
            let x1 = (x0 + 1).min(sw as usize - 1);
            for c in 0..cn {
                let p00 = src[(y0 * sw as usize + x0) * cn + c] as i32;
                let p01 = src[(y0 * sw as usize + x1) * cn + c] as i32;
                let p10 = src[(y1 * sw as usize + x0) * cn + c] as i32;
                let p11 = src[(y1 * sw as usize + x1) * cn + c] as i32;
                let v = p00 * cy0 * cx0
                    + p01 * cy0 * cx1
                    + p10 * cy1 * cx0
                    + p11 * cy1 * cx1
                    + (1 << (SHIFT - 1));
                let di = ((dy as usize * dw as usize) + dx) * cn + c;
                dst[di] = (v >> SHIFT).clamp(0, 255) as u8;
            }
        }
    }
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_letterbox_padding_is_114() {
        let frame = vec![0u8; 320 * 240 * 3];
        let (data, g) = letterbox_nchw(&frame, 320, 240, 640).unwrap();
        assert_eq!(data.len(), 3 * 640 * 640);
        // 320x240 → r=2, nw=640, nh=480, dy=80：顶部 80 行是 114 灰边
        assert!((data[0] - 114.0 / 255.0).abs() < 1e-6);
        // 画布中心落在缩放后的图像内（黑）
        let center = (2 * 640 * 640) + 320 * 640 + 320;
        assert!((data[center]).abs() < 1e-6);
        assert!((g.dy - 80.0).abs() < 1e-6);
        assert!((g.dx - 0.0).abs() < 1e-6);
    }

    #[test]
    fn rows_keep_legacy_yolo_layouts() {
        let values = std::array::from_fn::<_, 42, _>(|i| i as f32);
        let channel_major = OutputRows::new(OutputFormat::Yolo, &[1, 6, 7], &values).unwrap();
        assert!(!channel_major.yolo_nms_free);
        assert_eq!(channel_major.get(1, 4), 29.0);
        let row_major = OutputRows::new(OutputFormat::Yolo, &[1, 7, 6], &values).unwrap();
        assert!(row_major.yolo_nms_free);
        assert_eq!(row_major.get(1, 4), 10.0);
        let row_major = OutputRows::new(OutputFormat::Yolo, &[1, 7, 5], &values[..35]).unwrap();
        assert!(!row_major.yolo_nms_free);
        assert_eq!(row_major.get(1, 4), 9.0);
    }

    #[test]
    fn rtdetr_six_columns_use_the_declared_format_and_original_aspect_ratio() {
        let geometry = BoxGeometry::Normalized { w: 1280, h: 720 };
        let values = [0.5, 0.5, 0.5, 0.5, 0.75, 1.0];
        let topk = OutputRows::new(OutputFormat::RtdetrTopk, &[1, 1, 6], &values).unwrap();
        let legacy = OutputRows::new(OutputFormat::RtdetrScores, &[1, 1, 6], &values).unwrap();
        for (rows, score) in [(topk, 0.75), (legacy, 1.0)] {
            assert_eq!(
                rows.decode(&geometry, 1, 0.5, 0.0, 0.5),
                [Det {
                    box_: [320.0, 180.0, 960.0, 540.0],
                    score
                }]
            );
            assert!(rows.decode(&geometry, 0, 0.5, 0.0, 0.5).is_empty());
        }
        let bgr = [0, 64, 128, 0, 64, 128];
        let tensor = stretch_nchw(&bgr, 2, 1, 2, 4).unwrap();
        assert_eq!(&tensor[..8], &[128.0 / 255.0; 8]);
        assert_eq!(&tensor[8..16], &[64.0 / 255.0; 8]);
        assert_eq!(&tensor[16..], &[0.0; 8]);
    }

    #[test]
    fn input_dimensions_follow_tensor_and_dynamic_fallback() {
        assert_eq!(input_size(&[1, 3, 720, 1280], 640).unwrap(), (1280, 720));
        assert_eq!(input_size(&[-1, 3, -1, 960], 640).unwrap(), (960, 640));
        assert!(input_size(&[1, 720, 1280, 3], 640).is_err());
        assert!(OutputRows::new(OutputFormat::RtdetrTopk, &[1, 6, 8400], &[]).is_err());
    }

    #[test]
    fn detect_runs_repo_model_on_blank_frame() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let model = repo.join("m0-spike").join("yolo11n.onnx");
        if !model.exists() {
            eprintln!("skip: 仓库模型不存在");
            return;
        }
        let mut m =
            YoloModel::load(&model, 640, 0.4, 0.5, 1200.0, None, false).expect("模型可加载");
        let frame = vec![114u8; 640 * 640 * 3];
        let dets = m.detect(&frame, 640, 640).expect("推理可执行");
        assert!(dets.is_empty(), "纯灰图不应检出目标，实得 {dets:?}");
        let mut second = YoloModel::load(&model, 640, 0.4, 0.5, 1200.0, None, false)
            .expect("同一进程应能载入第二个会话");
        assert!(second.detect(&frame, 640, 640).unwrap().is_empty());
    }

    #[test]
    fn ep_used_updates_dynamically() {
        set_ep_used("DirectML(GPU)");
        assert_eq!(ep_used().as_deref(), Some("DirectML(GPU)"));
        set_ep_used("CPU");
        assert_eq!(ep_used().as_deref(), Some("CPU"));
    }

    #[cfg(feature = "ort")]
    #[test]
    fn dml_lock_serializes_concurrent_callers() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let a = Arc::clone(&active);
            let m = Arc::clone(&max_seen);
            handles.push(std::thread::spawn(move || {
                for _ in 0..10 {
                    let _g = dml_lock().lock().unwrap_or_else(|e| e.into_inner());
                    let cur = a.fetch_add(1, Ordering::SeqCst) + 1;
                    m.fetch_max(cur, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    a.fetch_sub(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(max_seen.load(Ordering::SeqCst), 1, "DML_LOCK 必须保证串行");
    }
}
