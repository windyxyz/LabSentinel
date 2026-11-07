//! T-321（L1）黄金语料运行器（Rust 侧）：`ls-analyzer --golden <video> --out <jsonl>`。
//!
//! 与 Python `tools/golden_run.py` 同口径（76 文档 §11）：
//! - 按 analysis_fps 抽帧（步长 = round(vfps/analysis_fps)，按帧序号而非墙钟）；
//! - 帧 ts = 固定合成基准 2026-01-01 + 原视频 PTS（第 i 个分析帧对应原视频
//!   第 (i-1)*step+1 帧 → ts_s = (i-1)*step / vfps）；
//! - 全画幅 zone「全画幅」+ enter/exit/enter_no_ppe 三规则（cooldown=0）；
//! - 事件 JSONL 字段与 Python 一致（camera/type/level/track_id/detail/ts）。
//!
//! 用法：cargo run -p ls-analyzer -- --golden <video> --out <jsonl> [--fps 5] [--model <onnx>]

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;

use crate::pipeline;
use crate::pipeline::{CameraPipeline, CameraSpec, PersonDetection, PpeStatus, RuleSpec, ZoneSpec};

const FULL_ZONE: &[(i64, i64)] = &[(0, 0), (1280, 0), (1280, 719), (0, 719)];

pub struct GoldenArgs {
    pub video: PathBuf,
    pub out: PathBuf,
    pub fps: f32,
    pub model: Option<PathBuf>,
    pub no_model: bool,
    /// L1 对齐诊断：每分析帧检测框 JSONL 追加写（--dump-dets <path>）
    pub dump_dets: Option<PathBuf>,
}

/// 构造与 Python golden_run 相同的配置：全画幅 zone + 三规则（cooldown=0）。
fn golden_pipeline(camera_id: i64, fps: f32) -> CameraPipeline {
    CameraPipeline::new(
        CameraSpec {
            id: camera_id,
            name: "golden".into(),
            source: String::new(),
            type_: "file".into(),
            analysis_fps: fps,
            file_loop: false,
        },
        vec![ZoneSpec {
            id: 1,
            camera_id,
            name: "全画幅".into(),
            points: FULL_ZONE.to_vec(),
        }],
        vec![
            golden_rule(1, "enter"),
            golden_rule(2, "exit"),
            golden_rule(3, "enter_no_ppe"),
        ],
        Default::default(),
        2,
        3,
    )
}

fn golden_rule(id: i64, behavior: &str) -> RuleSpec {
    RuleSpec {
        id,
        camera_id: 1,
        zone_id: 1,
        behavior: behavior.into(),
        level: 2,
        cooldown_s: 0,
        time_start: "00:00".into(),
        time_end: "23:59".into(),
        weekdays: String::new(),
        threshold: 0,
        enabled: true,
        holiday_only: false,
    }
}

fn frame_ts(index: u64, step: u64, vfps: f64) -> String {
    // 第 index 个分析帧（1-based）对应原视频第 (index-1)*step+1 帧；
    // Python 的 POS_MSEC 实测 = (index-1)*step/vfps 秒（0, 0.2, 0.4...），等价复现。
    // （VFR 视频的精确对齐走 frame_ts_pts / 逐帧 PTS 表；此函数为无 PTS 表时的回退。）
    let secs = ((index.saturating_sub(1)) as f64) * step as f64 / vfps.max(0.001);
    let base = NaiveDateTime::parse_from_str("2026-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S")
        .expect("固定基准合法");
    let dt = base + chrono::TimeDelta::milliseconds((secs * 1000.0).round() as i64);
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn ts_from_secs(secs: f64) -> String {
    let base = NaiveDateTime::parse_from_str("2026-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S")
        .expect("固定基准合法");
    let dt = base + chrono::TimeDelta::milliseconds((secs * 1000.0).round() as i64);
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// ffprobe 逐帧 PTS（秒）：与 cv2 POS_MSEC 同源（容器 PTS，VFR 准确）。
/// 输出顺序 = 解码顺序（与 ffmpeg rawvideo 管道/Python cap.read 一致）。
fn probe_frame_pts(video: &Path) -> Result<Vec<f64>, String> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=pts_time",
            "-of",
            "csv=p=0",
        ])
        .arg(video)
        .output()
        .map_err(|e| format!("ffprobe 不可用: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pts = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let v = l.split(',').next().unwrap_or("0");
        let secs = v.parse::<f64>().unwrap_or(0.0); // "N/A"/空 → 0（首帧无 PTS）
        pts.push(secs);
    }
    if pts.is_empty() {
        return Err("ffprobe 逐帧 PTS 为空".into());
    }
    Ok(pts)
}

/// 跑黄金语料：返回事件条数（写 JSONL 到 out）。
pub fn run_golden(args: &GoldenArgs) -> Result<usize, String> {
    let probe = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate,width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(&args.video)
        .output()
        .map_err(|e| format!("ffprobe 不可用: {e}"))?;
    // csv 输出：<width>,<height>,<rate>（ffprobe 按内部字段序输出）
    let line = String::from_utf8_lossy(&probe.stdout);
    let first = line.lines().next().ok_or("ffprobe 输出为空")?;
    let mut it = first.trim().split(',');
    let src_w: u32 = it
        .next()
        .and_then(|v| v.trim().parse().ok())
        .ok_or("ffprobe 输出异常（宽）")?;
    let src_h: u32 = it
        .next()
        .and_then(|v| v.trim().parse().ok())
        .ok_or("ffprobe 输出异常（高）")?;
    let (num, den) = it
        .next()
        .and_then(|r| r.trim().split_once('/'))
        .ok_or("ffprobe 输出异常（帧率）")?;
    let vfps: f64 = num.trim().parse::<f64>().map_err(|e| e.to_string())?
        / den.trim().parse::<f64>().map_err(|e| e.to_string())?;
    if vfps <= 0.0 {
        return Err("视频帧率非法".into());
    }
    let step = (vfps / args.fps.max(0.1) as f64).round().max(1.0) as u64;
    // 逐帧 PTS（与 Python POS_MSEC 同源）；失败回退 帧序×(1/fps) 近似。
    let pts_table = probe_frame_pts(&args.video)
        .map_err(|e| eprintln!("[golden-rs] 逐帧 PTS 不可用（{e}），回退帧序计算"))
        .unwrap_or_default();

    let mut ing = ls_ingest::Ingest::new(ls_ingest::IngestConfig {
        source: args.video.to_string_lossy().to_string(),
        // 黄金语料对齐 Python（cv2）两级缩放口径：analysis_fps=0 → ffmpeg **纯解码**
        //（无 fps/scale 滤镜，帧序/PTS/源分辨率不重排），输出源分辨率原始帧；
        // 第一级 720p 缩放由本循环用 resize_u8_linear_cv2（cv2 逐位复刻）完成——
        // ffmpeg swscale 与 cv2.resize 的插值差是此前 conf 偏差的第三级根因。
        analysis_fps: 0.0,
        width: src_w,
        height: src_h,
        file_loop: false,
        rtsp_transport_tcp: true,
    });
    ing.spawn().map_err(|e| format!("ffmpeg 取流失败: {e}"))?;

    let mut detector = make_golden_detector(args)?;
    let mut pipe = golden_pipeline(1, args.fps);
    let out = std::io::BufWriter::new(
        std::fs::File::create(&args.out).map_err(|e| format!("输出不可写: {e}"))?,
    );
    let mut writer = out;
    let mut fi: u64 = 0; // 原视频帧序号（1-based）
    let mut analyzed: u64 = 0;
    let mut events = 0usize;
    while let Some(frame) = ing.read_frame().map_err(|e| format!("取流中断: {e}"))? {
        fi += 1;
        if !(fi - 1).is_multiple_of(step) {
            continue; // 与 Python golden_run 同口径：按帧序号抽帧
        }
        analyzed += 1;
        // 第一级缩放（对齐 Python golden_run 的 cv2.resize(frame, (1280, 720))）：
        // 源帧 != 720p 时用 cv2 逐位复刻缩放；已是 720p 则原样。
        let work = if frame.width == 1280 && frame.height == 720 {
            frame
        } else {
            let resized =
                ls_infer::yolo::resize_u8_linear_cv2(&frame.data, frame.width, frame.height, 1280, 720, 3);
            ls_ingest::Frame {
                width: 1280,
                height: 720,
                data: resized,
                index: frame.index,
            }
        };
        let dets: Vec<PersonDetection> = detector
            .detect(&work)
            .into_iter()
            .map(|d| PersonDetection {
                box_: d.box_,
                score: d.score,
                ppe: PpeStatus::Unknown,
            })
            .collect();
        let ts = if analyzed > 0 && ((analyzed as usize - 1) * step as usize) < pts_table.len() {
            // 分析帧 i → 原始帧 (i-1)*step+1（0-based 索引 (i-1)*step）；
            // Python POS_MSEC = 该帧的容器 PTS，逐位同源。
            ts_from_secs(pts_table[(analyzed as usize - 1) * step as usize])
        } else {
            frame_ts(analyzed, step, vfps)
        };
        if let Some(dump) = &args.dump_dets {
            let mut df = std::io::BufWriter::new(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(dump)
                    .map_err(|e| e.to_string())?,
            );
            let boxes: Vec<Vec<f32>> = dets.iter().map(|d| d.box_.to_vec()).collect();
            let scores: Vec<f32> = dets.iter().map(|d| d.score).collect();
            writeln!(
                df,
                "{}",
                serde_json::json!({"frame": analyzed, "boxes": boxes, "scores": scores})
            )
            .map_err(|e| e.to_string())?;
            df.flush().map_err(|e| e.to_string())?;
        }
        for cand in pipe.process(&dets, &ts) {
            events += 1;
            let line = serde_json::json!({
                "camera": cand.camera_id, "type": cand.behavior, "level": cand.level,
                "track_id": cand.track_id, "detail": cand.detail, "ts": cand.ts_iso,
            });
            writeln!(writer, "{line}").map_err(|e| e.to_string())?;
        }
    }
    writer.flush().map_err(|e| e.to_string())?;
    println!(
        "[golden-rs] video={} analyzed={analyzed} events={events} → {}",
        args.video.display(),
        args.out.display()
    );
    Ok(events)
}

/// 检测器构造（与 sidecar 相同；--no-model 恒空）。
fn make_golden_detector(args: &GoldenArgs) -> Result<Box<dyn pipeline::Detector>, String> {
    if args.no_model {
        return Ok(Box::new(crate::pipeline::NullDetector));
    }
    let path: PathBuf = match args.model.as_ref() {
        Some(p) => p.clone(),
        None => std::env::var("LS_MODEL_FP32")
            .ok()
            .map(PathBuf::from)
            .ok_or("未找到模型（--model / LS_MODEL_FP32）")?,
    };
    if !path.is_file() {
        return Err(format!("模型不存在: {}", path.display()));
    }
    let model = ls_infer::yolo::YoloModel::load(&path, 640, 0.40, 0.50, 1200.0, None)
        .map_err(|e| format!("模型加载失败: {e}"))?;
    Ok(Box::new(GoldenDetector { model }))
}

struct GoldenDetector {
    model: ls_infer::yolo::YoloModel,
}

impl pipeline::Detector for GoldenDetector {
    fn detect(&mut self, frame: &ls_ingest::Frame) -> Vec<pipeline::PersonDetection> {
        match self.model.detect(&frame.data, frame.width, frame.height) {
            Ok(dets) => dets
                .into_iter()
                .map(|d| pipeline::PersonDetection {
                    box_: d.box_,
                    score: d.score,
                    ppe: PpeStatus::Unknown,
                })
                .collect(),
            Err(e) => {
                eprintln!("[golden-rs] 推理失败（本帧跳过）: {e}");
                Vec::new()
            }
        }
    }
}
