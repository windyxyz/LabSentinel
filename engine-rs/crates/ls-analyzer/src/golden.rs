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
use std::path::PathBuf;

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
    let secs = ((index.saturating_sub(1)) as f64) * step as f64 / vfps.max(0.001);
    let base = NaiveDateTime::parse_from_str("2026-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S")
        .expect("固定基准合法");
    let dt = base + chrono::TimeDelta::milliseconds((secs * 1000.0).round() as i64);
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
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
            "stream=r_frame_rate",
            "-of",
            "csv=p=0",
        ])
        .arg(&args.video)
        .output()
        .map_err(|e| format!("ffprobe 不可用: {e}"))?;
    let rate = String::from_utf8_lossy(&probe.stdout);
    let (num, den) = rate
        .trim()
        .split_once('/')
        .ok_or("ffprobe 输出异常（帧率）")?;
    let vfps: f64 = num.trim().parse::<f64>().map_err(|e| e.to_string())?
        / den.trim().parse::<f64>().map_err(|e| e.to_string())?;
    if vfps <= 0.0 {
        return Err("视频帧率非法".into());
    }
    let step = (vfps / args.fps.max(0.1) as f64).round().max(1.0) as u64;

    let mut ing = ls_ingest::Ingest::new(ls_ingest::IngestConfig {
        source: args.video.to_string_lossy().to_string(),
        // 黄金语料对齐 Python 逐帧口径：不交给 ffmpeg fps 滤镜抽帧（否则双重抽帧），
        // 拿全部原始帧后由本循环按 step 取（step = round(vfps/analysis_fps)）。
        analysis_fps: vfps as f32,
        width: 1280,
        height: 720,
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
        let dets: Vec<PersonDetection> = detector
            .detect(&frame)
            .into_iter()
            .map(|d| PersonDetection {
                box_: d.box_,
                score: d.score,
                ppe: PpeStatus::Unknown,
            })
            .collect();
        let ts = frame_ts(analyzed, step, vfps);
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
    let model = ls_infer::yolo::YoloModel::load(&path, 640, 0.40, 0.50, 1200.0)
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
