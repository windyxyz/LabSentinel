//! CV measurement only. DirectML uses the same single GPU worker as the server.

use ls_infer::post::Det;
use ls_infer::yolo::{ep_used, Backend, YoloModel};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Barrier;
use std::time::Instant;

#[cfg(windows)]
#[path = "vulkan_spike/ncnn.rs"]
mod ncnn;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

enum Runtime {
    Ort(bool),
    #[cfg(windows)]
    Ncnn {
        api: std::sync::Arc<ncnn::Api>,
        adapter: i32,
        fp16: bool,
    },
}

impl Runtime {
    fn name(&self) -> &'static str {
        match self {
            Self::Ort(false) => "CPU",
            Self::Ort(true) => "DirectML(GPU)",
            #[cfg(windows)]
            Self::Ncnn { fp16: false, .. } => "ncnn/Vulkan FP32",
            #[cfg(windows)]
            Self::Ncnn { fp16: true, .. } => "ncnn/Vulkan FP16",
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
            Self::Ncnn { api, adapter, fp16 } => {
                let net =
                    ncnn::Model::load(std::sync::Arc::clone(api), model_path, *adapter, *fp16)?;
                Ok(YoloModel::spike_backend(Box::new(net)))
            }
        }
    }
}

fn boxes(dets: Vec<Det>) -> Vec<Value> {
    dets.into_iter()
        .map(|det| json!({ "xyxy": det.box_, "score": det.score }))
        .collect()
}

fn run(
    model_path: &Path,
    frames: &[Vec<u8>],
    runtime: &Runtime,
    streams: usize,
    iterations: usize,
) -> Result<Value, String> {
    let mut models = Vec::with_capacity(streams);
    for _ in 0..streams {
        let mut model = runtime.load(model_path)?;
        for frame in frames {
            model.detect(frame, WIDTH, HEIGHT)?;
        }
        models.push(model);
    }

    let predictions = frames
        .iter()
        .map(|frame| models[0].detect(frame, WIDTH, HEIGHT).map(boxes))
        .collect::<Result<Vec<_>, _>>()?;

    let barrier = Barrier::new(streams + 1);
    let (elapsed, mut samples) = std::thread::scope(|scope| {
        let handles: Vec<_> = models
            .into_iter()
            .map(|mut model| {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let mut samples = Vec::with_capacity(iterations);
                    for frame in frames.iter().cycle().take(iterations) {
                        let start = Instant::now();
                        model.detect(frame, WIDTH, HEIGHT)?;
                        samples.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                    Ok::<_, String>(samples)
                })
            })
            .collect();
        let started = Instant::now();
        barrier.wait();
        let mut samples = Vec::with_capacity(iterations * streams);
        for handle in handles {
            samples.extend(handle.join().map_err(|_| "inference thread failed")??);
        }
        Ok::<_, String>((started.elapsed().as_secs_f64(), samples))
    })?;
    samples.sort_unstable_by(f64::total_cmp);
    let percentile = |value: f64| samples[(samples.len() as f64 * value).ceil() as usize - 1];
    Ok(json!({
        "provider": runtime.name(),
        "streams": streams,
        "frames": samples.len(),
        "elapsed_s": elapsed,
        "fps": samples.len() as f64 / elapsed,
        "p50_ms": percentile(0.50),
        "p95_ms": percentile(0.95),
        "predictions": predictions,
    }))
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [provider, model, iterations, frames @ ..] = args.as_slice() else {
        return Err("usage: vulkan_spike <cpu|dml|ncnn|ncnn-fp16> <model.onnx|model.ncnn> <iterations> <1280x720.bgr>...".into());
    };
    let runtime = match provider.to_str() {
        Some("cpu") => Runtime::Ort(false),
        Some("dml") => Runtime::Ort(true),
        #[cfg(windows)]
        Some("ncnn" | "ncnn-fp16") => {
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
            Runtime::Ncnn {
                api: ncnn::Api::load(&library)?,
                adapter,
                fp16: provider == "ncnn-fp16",
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
    for streams in [1, 4] {
        println!(
            "{}",
            run(Path::new(model), &frames, &runtime, streams, iterations)?
        );
    }
    Ok(())
}
