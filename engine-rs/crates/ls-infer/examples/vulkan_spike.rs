//! CV measurement only. DirectML uses the same single GPU worker as the server.

use ls_infer::yolo::{ep_used, Backend, YoloModel};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Barrier, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use ls_infer::ncnn;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const PPE_CLASSES: [(usize, f32, f32); 2] = [(2, 0.25, 0.0), (6, 0.25, 0.0)];

struct Soak<'a> {
    seconds: u32,
    fps: u32,
    ppe: &'a Path,
}

enum Runtime {
    Ort(bool),
    #[cfg(windows)]
    Ncnn {
        api: std::sync::Arc<ncnn::Api>,
        adapter: i32,
        adapter_count: i32,
        precision: ncnn::Precision,
    },
}

impl Runtime {
    fn name(&self) -> &'static str {
        match self {
            Self::Ort(false) => "CPU",
            Self::Ort(true) => "DirectML(GPU)",
            #[cfg(windows)]
            Self::Ncnn { precision, .. } => precision.label(),
        }
    }

    fn load(&self, model_path: &Path) -> Result<YoloModel, String> {
        match self {
            Self::Ort(gpu) => {
                let model =
                    YoloModel::load(model_path, 640, 0.25, 0.45, 0.0, Some(Backend::Ort), *gpu)?;
                let provider = ep_used();
                if provider.as_deref() != Some(self.name()) {
                    return Err(format!(
                        "requested {}, model loaded on {provider:?}",
                        self.name()
                    ));
                }
                Ok(model)
            }
            #[cfg(windows)]
            Self::Ncnn {
                api,
                adapter,
                precision,
                ..
            } => {
                let net = ncnn::Model::load(
                    std::sync::Arc::clone(api),
                    model_path,
                    *adapter,
                    *precision,
                )?;
                Ok(YoloModel::spike_backend(Box::new(net)))
            }
        }
    }

    fn gpu_selection(&self) -> Value {
        match self {
            #[cfg(windows)]
            Self::Ncnn {
                adapter,
                adapter_count,
                ..
            } => json!({"index": adapter, "count": adapter_count}),
            Self::Ort(_) => Value::Null,
        }
    }
}

fn run(
    model_path: &Path,
    frames: &[Vec<u8>],
    runtime: &Runtime,
    streams: usize,
    iterations: usize,
    comparison_classes: usize,
    soak: Option<&Soak<'_>>,
) -> Result<Value, String> {
    let period = soak.map_or(Duration::ZERO, |s| Duration::from_secs(1) / s.fps);
    let slots = soak.map_or(iterations, |s| (s.seconds * s.fps) as usize);
    let mut models = Vec::with_capacity(streams);
    for _ in 0..streams {
        let mut model = runtime.load(model_path)?;
        let mut ppe = soak.map(|config| runtime.load(config.ppe)).transpose()?;
        for frame in frames {
            model.detect(frame, WIDTH, HEIGHT)?;
            if let Some(ppe) = ppe.as_mut() {
                ppe.detect_classes(frame, WIDTH, HEIGHT, &PPE_CLASSES)?;
            }
        }
        models.push((model, ppe));
    }

    // Decode all the requested classes outside the timed section. The timed path
    // keeps the original single-class product measurement comparable.
    let specs: Vec<_> = (0..comparison_classes).map(|id| (id, 0.25, 0.0)).collect();
    let predictions = frames
        .iter()
        .map(|frame| {
            models[0].0.detect_classes(frame, WIDTH, HEIGHT, &specs).map(|groups| {
                groups.into_iter().enumerate().flat_map(|(class_id, items)| {
                    items.into_iter().map(move |det| json!({"class_id": class_id, "xyxy": det.box_, "score": det.score}))
                }).collect::<Vec<_>>()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let barrier = Barrier::new(streams + 1);
    let started = OnceLock::new();
    let cancelled = AtomicBool::new(false);
    let (elapsed, started_at_ms, finished_at_ms, mut samples, lanes) = std::thread::scope(
        |scope| {
            let handles: Vec<_> = models
            .into_iter()
            .enumerate()
            .map(|(lane, (mut model, mut ppe))| {
                let barrier = &barrier;
                let started = &started;
                let cancelled = &cancelled;
                scope.spawn(move || {
                    barrier.wait();
                    let origin: Instant = *started.get().expect("measurement origin");
                    let mut samples = Vec::with_capacity(slots);
                    let mut next = 0usize;
                    let mut late = 0usize;
                    let mut heartbeat = Duration::from_secs(60);
                    while next < slots && !cancelled.load(Ordering::Acquire) {
                        if !period.is_zero() {
                            let due = (origin.elapsed().as_nanos() / period.as_nanos()) as usize;
                            next = next.max(due);
                            if next >= slots {
                                break;
                            }
                            let deadline = origin + period * next as u32;
                            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                            if cancelled.load(Ordering::Acquire) {
                                break;
                            }
                        }
                        let frame = &frames[(next + lane) % frames.len()];
                        let sample_start = Instant::now();
                        let infer = (|| {
                            model.detect(frame, WIDTH, HEIGHT)?;
                            if let Some(ppe) = ppe.as_mut() {
                                ppe.detect_classes(frame, WIDTH, HEIGHT, &PPE_CLASSES)?;
                            }
                            Ok::<_, String>(())
                        })();
                        if let Err(error) = infer {
                            cancelled.store(true, Ordering::Release);
                            return Err(error);
                        }
                        let sample = sample_start.elapsed();
                        if !period.is_zero() && origin.elapsed() > period * (next + 1) as u32 {
                            late += 1;
                        }
                        samples.push(sample.as_secs_f64() * 1000.0);
                        next += 1;
                        if !period.is_zero() && origin.elapsed() >= heartbeat {
                            eprintln!("LS_VULKAN_SPIKE_PROGRESS lane={} frames={} missed={} late={late}", lane + 1, samples.len(), next - samples.len());
                            heartbeat += Duration::from_secs(60);
                        }
                    }
                    if !period.is_zero() && !cancelled.load(Ordering::Acquire) {
                        let end = origin + period * slots as u32;
                        std::thread::sleep(end.saturating_duration_since(Instant::now()));
                    }
                    Ok((samples, late))
                })
            })
            .collect();
            let origin = Instant::now();
            started.set(origin).expect("single measurement origin");
            let started_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            if std::env::var_os("LS_VULKAN_SPIKE_PHASES").is_some() {
                eprintln!("LS_VULKAN_SPIKE_READY {streams} {started_at_ms}");
            }
            barrier.wait();
            let mut samples = Vec::with_capacity(slots * streams);
            let mut lanes = Vec::with_capacity(streams);
            for handle in handles {
                let (lane_samples, late) =
                    handle.join().map_err(|_| "inference thread failed")??;
                lanes.push(json!({"frames": lane_samples.len(), "missed": slots - lane_samples.len(), "late": late}));
                samples.extend(lane_samples);
            }
            let finished_at_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            Ok::<_, String>((
                origin.elapsed().as_secs_f64(),
                started_at_ms,
                finished_at_ms,
                samples,
                lanes,
            ))
        },
    )?;
    if samples.is_empty() {
        return Err("measurement completed without inference samples".into());
    }
    samples.sort_unstable_by(f64::total_cmp);
    let percentile = |value: f64| samples[(samples.len() as f64 * value).ceil() as usize - 1];
    Ok(json!({
        "provider": runtime.name(),
        "streams": streams,
        "frames": samples.len(),
        "elapsed_s": elapsed,
        "started_at_ms": started_at_ms,
        "finished_at_ms": finished_at_ms,
        "fps": samples.len() as f64 / elapsed,
        "lanes": lanes,
        "soak_seconds": soak.map(|s| s.seconds),
        "target_fps_per_lane": soak.map(|s| s.fps),
        "p50_ms": percentile(0.50),
        "p95_ms": percentile(0.95),
        "comparison_classes": comparison_classes,
        "gpu_selection": runtime.gpu_selection(),
        "predictions": predictions,
    }))
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(windows)]
    if let [flag, root, cache] = args.as_slice() {
        if flag == "--verify-package" {
            ls_infer::vulkan_assets::verify_package(Path::new(root), Path::new(cache))?;
            println!("Vulkan package: verified both models and the shared DLL");
            return Ok(());
        }
    }
    let (comparison_classes, positionals) = match args.as_slice() {
        [flag, count, remaining @ ..] if flag == "--classes" => {
            let count = count
                .to_str()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|count| (1..=80).contains(count))
                .ok_or("--classes must be within 1..=80")?;
            (count, remaining)
        }
        _ => (1, args.as_slice()),
    };
    let (soak, positionals) = match positionals {
        [flag, fps, seconds, ppe, remaining @ ..] if flag == "--soak" => {
            let fps = fps
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|fps| (1..=30).contains(fps))
                .ok_or("soak FPS must be within 1..=30")?;
            let seconds = seconds
                .to_str()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|seconds| (1..=14_400).contains(seconds))
                .ok_or("soak seconds must be within 1..=14400")?;
            (
                Some(Soak {
                    seconds,
                    fps,
                    ppe: Path::new(ppe),
                }),
                remaining,
            )
        }
        _ => (None, positionals),
    };
    let [provider, model, iterations, frames @ ..] = positionals else {
        return Err("usage: vulkan_spike [--classes N] [--soak FPS SECONDS PPE_MODEL] <cpu|dml|ncnn|ncnn-fp16-storage|ncnn-fp16> <model.onnx|model.ncnn> <iterations> <1280x720.bgr>...".into());
    };
    let runtime = match provider.to_str() {
        Some("cpu") => Runtime::Ort(false),
        Some("dml") => Runtime::Ort(true),
        #[cfg(windows)]
        Some(name @ ("ncnn" | "ncnn-fp16-storage" | "ncnn-fp16")) => {
            let library = std::env::var_os("LS_NCNN_DLL").ok_or("set LS_NCNN_DLL to ncnn.dll")?;
            let library = Path::new(&library)
                .canonicalize()
                .map_err(|error| error.to_string())?;
            let adapter = std::env::var("LS_NCNN_ADAPTER")
                .unwrap_or_else(|_| "0".into())
                .parse::<i32>()
                .ok()
                .filter(|&gpu| gpu >= 0)
                .ok_or("invalid ncnn adapter index")?;
            let api = ncnn::Api::load(&library)?;
            let adapter_count = api.validate_adapter(adapter)?;
            Runtime::Ncnn {
                api,
                adapter,
                adapter_count,
                precision: match name {
                    "ncnn" => ncnn::Precision::Fp32,
                    "ncnn-fp16-storage" => ncnn::Precision::Fp16Storage,
                    _ => ncnn::Precision::Fp16,
                },
            }
        }
        _ => return Err("unknown runtime for this platform".into()),
    };
    let iterations: usize = iterations
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|&count| count > 0)
        .ok_or("expected positive iteration count")?;
    if frames.is_empty() {
        return Err("at least one raw BGR frame is required".into());
    }
    let frames = frames
        .iter()
        .map(|path| {
            let data = std::fs::read(path).map_err(|error| format!("{path:?}: {error}"))?;
            if data.len() != (WIDTH * HEIGHT * 3) as usize {
                return Err(format!("{path:?}: expected one {WIDTH}x{HEIGHT} BGR frame"));
            }
            Ok(data)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for &streams in if soak.is_some() {
        &[4][..]
    } else {
        &[1, 4][..]
    } {
        println!(
            "{}",
            run(
                Path::new(model),
                &frames,
                &runtime,
                streams,
                iterations,
                comparison_classes,
                soak.as_ref(),
            )?
        );
    }
    Ok(())
}
