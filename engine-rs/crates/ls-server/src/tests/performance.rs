use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ls_ingest::child_guard::ChildGuard;
use serde_json::json;

use super::helpers::test_state_cfg_dir;
use crate::state::Shared;
use crate::worker::{start_worker, stop_all_workers, WorkerConfig};

struct RunningWorkers(Shared);

impl Drop for RunningWorkers {
    fn drop(&mut self) {
        stop_all_workers(&self.0);
    }
}

fn package_assets() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("LS_INSTALL_DIR")
            .expect("LS_INSTALL_DIR must point to the staged package"),
    );
    for required in [
        "tools/ffmpeg/ffmpeg.exe",
        "tools/go2rtc.exe",
        "models/yolo11n.onnx",
        "models/yolo11n_ppe.onnx",
    ] {
        assert!(
            path.join(required).is_file(),
            "missing benchmark asset {required}"
        );
    }
    path
}

fn number_setting(key: &str, fallback: u64) -> u64 {
    std::env::var(key)
        .ok()
        .map(|value| value.parse().expect("numeric benchmark option"))
        .unwrap_or(fallback)
}

fn fixture(assets: &Path, work: &Path) -> PathBuf {
    let person = work.join("person.jpg");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../Resources/acceptance/imgs/person-frame.jpg"),
        &person,
    )
    .unwrap();
    let path = work.join("benchmark.mp4");
    // The five clips last four seconds each. Their order is also used by the truth replay.
    let filters = "[1:v]scale=1920:1080:force_original_aspect_ratio=decrease,\
                   pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p,trim=duration=4,split=3[person][dark][far];\
                   [dark]eq=brightness=-0.19[lowlight];\
                   [far]scale=960:540,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=gray[distant];\
                   [0:v][person][lowlight][distant][2:v]concat=n=5:v=1:a=0[out]";
    let mut command = Command::new(assets.join("tools/ffmpeg/ffmpeg.exe"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let generated = command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=c=gray:s=1920x1080:r=12:d=4",
            "-loop",
            "1",
            "-framerate",
            "12",
            "-i",
        ])
        .arg(person)
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=1920x1080:r=12:d=4",
            "-filter_complex_threads",
            "1",
            "-filter_complex",
            filters,
            "-map",
            "[out]",
            "-t",
            "20",
            "-an",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "12",
            "-threads",
            "2",
        ])
        .arg(&path)
        .output()
        .expect("fixture encoder");
    assert!(
        generated.status.success(),
        "fixture encoder: {}",
        String::from_utf8_lossy(&generated.stderr)
    );
    path
}

fn reserve_local_ports() -> (u16, u16) {
    let api = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let rtsp = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    (
        api.local_addr().unwrap().port(),
        rtsp.local_addr().unwrap().port(),
    )
}

fn start_source(assets: &Path, work: &Path, api: u16, rtsp: u16) -> ChildGuard {
    let config = json!({
        "api": {"listen": format!("127.0.0.1:{api}")},
        "rtsp": {"listen": format!("127.0.0.1:{rtsp}")},
        "webrtc": {"listen": ""},
        "ffmpeg": {"bin": assets.join("tools/ffmpeg/ffmpeg.exe"), "file": "-re -stream_loop -1 -i {input}"},
        "streams": {"benchmark": "ffmpeg:benchmark.mp4#video=copy"},
    });
    let mut command = Command::new(assets.join("tools/go2rtc.exe"));
    command
        .args(["-c", &config.to_string()])
        .current_dir(work)
        .stdout(Stdio::null());
    let child = ChildGuard::spawn(command).expect("start benchmark RTSP source");
    assert!(
        wait_until(Duration::from_secs(8), || TcpStream::connect((
            "127.0.0.1",
            api
        ))
        .is_ok()),
        "benchmark RTSP gateway did not start"
    );
    child
}

fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[ignore = "native 1080p RTSP and model benchmark; requires the staged Windows package assets"]
fn windows_rtsp_benchmark() {
    let assets = package_assets();
    let cameras = number_setting("LS_BENCH_CAMERAS", 1);
    assert!([1, 4, 9].contains(&cameras), "use 1, 4 or 9 cameras");
    let duration = Duration::from_secs(number_setting("LS_BENCH_SECONDS", 30));
    assert!(!duration.is_zero());
    let ep = std::env::var("LS_BENCH_EP").unwrap_or_else(|_| "cpu".into());
    assert!(["cpu", "dml", "auto_vulkan"].contains(&ep.as_str()));
    assert!(
        ep != "auto_vulkan" || cfg!(all(windows, feature = "vulkan-cv")),
        "use --features vulkan-cv"
    );
    if ep == "auto_vulkan" {
        assert!(
            assets.join("vulkan/ncnn.dll.xz").is_file(),
            "use the staged Vulkan package"
        );
    }
    let work = tempfile::tempdir().unwrap();
    fixture(&assets, work.path());
    let db = work.path().join("benchmark.db");
    let db_path = db.to_str().unwrap();
    let store = ls_store::Store::open(db_path).unwrap();
    store.ensure_schema().unwrap();
    for (key, value) in [
        ("runtime.backend", ep.as_str()),
        ("detect.mode", "local"),
        ("ppe.enabled", "1"),
        ("perf.motion_gate_enabled", "0"),
        (
            "model.active",
            assets.join("models/yolo11n.onnx").to_str().unwrap(),
        ),
        (
            "ppe_model",
            assets.join("models/yolo11n_ppe.onnx").to_str().unwrap(),
        ),
    ] {
        store.setting_set(key, value).unwrap();
    }
    let (api, rtsp) = reserve_local_ports();
    let mut source = start_source(&assets, work.path(), api, rtsp);
    let st = test_state_cfg_dir(db_path, "", false, work.path().to_str().unwrap());
    let running = RunningWorkers(st.clone());
    let mut handles = Vec::new();
    for i in 0..cameras {
        let id = store
            .conn_add_camera(
                &format!("benchmark-{i}"),
                &format!("rtsp://127.0.0.1:{rtsp}/benchmark"),
                "rtsp",
            )
            .unwrap();
        let zone = store
            .add_zone(id, "benchmark", "[[0,0],[1279,0],[1279,719],[0,719]]")
            .unwrap();
        store
            .add_rule(id, Some(zone), "enter", 2, 5, None, None, 1, true)
            .unwrap();
        let mut cfg = WorkerConfig::from_db(&store, id).unwrap().unwrap();
        cfg.spec.analysis_fps = 5.0;
        cfg.evidence_fps = 10.0;
        assert!(cfg.detector.model_path.is_some() && cfg.ppe.is_some());
        start_worker(&st, cfg).unwrap();
        handles.push((id, st.workers.get(id).unwrap()));
    }
    let warm_ppe = std::env::var("LS_BENCH_WARM_PPE").as_deref() == Ok("1");
    let warmed = wait_until(Duration::from_secs(50), || {
        handles.iter().all(|(_, handle)| {
            let performance = handle.performance.lock().unwrap();
            performance.frames() >= 5
                && (!warm_ppe || performance.summary()["latency"]["person_ppe_ms"].is_object())
        })
    });
    if warm_ppe {
        assert!(warmed, "not every camera ran PPE during warmup");
    }
    let warmup: Vec<_> = handles
        .iter()
        .map(|(id, handle)| {
            let mut window = handle.performance.lock().unwrap();
            let mut summary = window.summary();
            summary["camera_id"] = json!(id);
            let input = window.input_kind;
            *window = Default::default();
            window.input_kind = input;
            summary
        })
        .collect();
    let before = store.count_events().unwrap();
    let first_day = ls_core::time::local_today_iso();
    let compliance_frames = |id, last_day: &str| {
        store
            .compliance_counts(&first_day, last_day, None, Some(&[id]))
            .unwrap()
            .frames
    };
    let before_compliance: Vec<_> = handles
        .iter()
        .map(|(id, _)| compliance_frames(*id, &first_day))
        .collect();
    let warm_pid_path = std::env::var_os("LS_BENCH_PID_FILE").map(PathBuf::from);
    if let Some(path) = warm_pid_path {
        std::fs::write(path, std::process::id().to_string()).unwrap();
    }
    let started = Instant::now();
    std::thread::sleep(duration);
    let measured_seconds = started.elapsed().as_secs_f64();
    let last_day = ls_core::time::local_today_iso();
    let per_camera: Vec<_> = handles
        .iter()
        .zip(before_compliance)
        .map(|((id, handle), before)| {
            let mut summary = handle.performance.lock().unwrap().summary();
            summary["camera_id"] = json!(id);
            summary["fps"] =
                json!(summary["frames"].as_u64().unwrap_or_default() as f64 / measured_seconds);
            summary["compliance_sample_frames"] = json!(compliance_frames(*id, &last_day) - before);
            summary
        })
        .collect();
    let events = store.count_events().unwrap() - before;
    let recovery_ms = if std::env::var("LS_BENCH_OUTAGE").as_deref() == Ok("1") {
        let started = Instant::now();
        source.stop();
        drop(source);
        std::thread::sleep(Duration::from_secs(2));
        let before: Vec<_> = handles
            .iter()
            .map(|(_, handle)| handle.performance.lock().unwrap().frames())
            .collect();
        source = start_source(&assets, work.path(), api, rtsp);
        wait_until(Duration::from_secs(35), || {
            handles
                .iter()
                .zip(&before)
                .all(|((_, handle), old)| handle.performance.lock().unwrap().frames() > *old)
        })
        .then(|| started.elapsed().as_millis())
    } else {
        None
    };
    let devices = st.workers.inference_devices();
    let report = json!({
        "scenario": "1080p H264 RTSP, empty/person/low light/distant/motion clips",
        "test_pid": std::process::id(), "requested_ep": ep, "last_loaded_ep": ls_infer::yolo::ep_used(),
        "seconds": measured_seconds, "all_cameras_warmed": warmed, "source_fps": 12, "target_analysis_fps": 5,
        "events": events, "recovery_ms": recovery_ms, "warmup_cameras": warmup,
        "cameras": per_camera, "model_devices": devices,
    });
    println!("{report}");
    if let Some(path) = std::env::var_os("LS_BENCH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    drop(running);
    drop(source);
    if ep == "auto_vulkan" {
        assert_eq!(
            devices.len(),
            cameras as usize,
            "missing active analysis sources"
        );
        assert!(
            devices
                .iter()
                .all(|row| row["person"] == "Vulkan(GPU)" && row["ppe"] == "Vulkan(GPU)"),
            "Vulkan unexpectedly fell back: {devices:?}"
        );
        assert!(
            per_camera.iter().all(|camera| {
                camera["vulkan_calls"].as_u64().unwrap_or_default()
                    > camera["inferred"].as_u64().unwrap_or_default()
                    && camera["latency"]["person_ppe_ms"]["p95"].is_number()
            }),
            "missing measured person + PPE Vulkan work on a camera"
        );
        if duration >= Duration::from_secs(15) {
            assert!(
                per_camera
                    .iter()
                    .all(|camera| camera["compliance_sample_frames"]
                        .as_i64()
                        .unwrap_or_default()
                        > 0),
                "a Vulkan camera did not persist periodic compliance samples"
            );
        }
    }
    assert!(
        per_camera
            .iter()
            .any(|camera| camera["inferred"].as_u64().unwrap_or_default() > 0),
        "no person inference observed"
    );
}

#[test]
#[ignore = "uses a real ONNX model and the local FFmpeg executable"]
fn packaged_person_model_detects_a_person_in_a_jpeg() {
    use ls_infer::yolo::{Backend, YoloModel};
    let model_path = PathBuf::from(
        std::env::var_os("LS_BENCH_PERSON_MODEL").expect("path to the release ONNX model"),
    );
    let sample = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../Resources/acceptance/imgs/person-frame.jpg");
    let (bgr, width, height) =
        ls_ingest::decode_image(&sample, 1280, 720).expect("decode the person fixture");
    let use_gpu = std::env::var("LS_BENCH_EP").as_deref() == Ok("dml");
    let mut model = YoloModel::load(
        &model_path,
        640,
        0.40,
        0.50,
        1200.0,
        Some(Backend::Ort),
        use_gpu,
    )
    .expect("load the release ONNX model");
    let people = model
        .detect(&bgr, width, height)
        .expect("run the model with a real picture");
    println!(
        "provider={:?} people={}",
        ls_infer::yolo::ep_used(),
        people.len()
    );
    assert!(
        !people.is_empty(),
        "known person fixture produced no detections"
    );
}

#[test]
#[ignore = "replays the performance fixture using both packaged ONNX models"]
fn windows_scene_truth_replay() {
    use ls_analyzer::detector::{make_detector, DetectorConfig};
    use ls_analyzer::pipeline::PpeStatus;
    use ls_analyzer::ppe::{make_ppe_runner, PpeConfig};

    let assets = package_assets();
    let work = tempfile::tempdir().unwrap();
    let clip = fixture(&assets, work.path());
    let use_gpu = std::env::var("LS_BENCH_EP").as_deref() == Ok("dml");
    let mut detector = make_detector(&DetectorConfig {
        model_path: Some(assets.join("models/yolo11n.onnx")),
        use_gpu,
        ..Default::default()
    });
    let mut ppe = make_ppe_runner(&PpeConfig {
        model_path: assets.join("models/yolo11n_ppe.onnx"),
        use_gpu,
        ..Default::default()
    })
    .expect("staged PPE model and class names");
    let mut source = ls_ingest::Ingest::new(ls_ingest::IngestConfig {
        source: clip.to_string_lossy().into_owned(),
        analysis_fps: 5.0,
        width: 1280,
        height: 720,
        file_loop: false,
        rtsp_transport_tcp: true,
    });
    source.spawn().unwrap();
    // Counts per clip: analyzed frames, frames with people, wearing, violation, unknown.
    let mut results = [[0u64; 5]; 5];
    while let Some(frame) = source.read_frame().unwrap() {
        let scene = (((frame.index - 1) / 20) as usize).min(results.len() - 1);
        let counts = &mut results[scene];
        counts[0] += 1;
        let people = detector.detect(&frame);
        counts[1] += u64::from(!people.is_empty());
        let boxes: Vec<_> = people.iter().map(|det| det.box_).collect();
        for status in ppe.statuses(&frame.data, frame.width, frame.height, &boxes) {
            let index = match status {
                PpeStatus::Wearing => 2,
                PpeStatus::NoPpe => 3,
                PpeStatus::Unknown => 4,
            };
            counts[index] += 1;
        }
    }
    let scenes = ["empty", "person", "low_light", "distant", "motion_pattern"];
    let truth = [false, true, true, true, false];
    let rows: Vec<_> = scenes.into_iter().zip(truth).zip(results)
        .map(|((scene, has_person), row)| json!({
            "scene": scene, "person_present": has_person, "frames": row[0], "frames_with_person": row[1],
            "ppe_wearing": row[2], "ppe_violation": row[3], "ppe_unknown": row[4],
        })).collect();
    let timings = ls_infer::yolo::take_inference_timings();
    let report = json!({"ep": ls_infer::yolo::ep_used(), "ort_calls": timings.calls,
        "directml_calls": timings.directml_calls, "scenes": rows});
    println!("{report}");
    if let Some(path) = std::env::var_os("LS_BENCH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert!(
        results.iter().all(|row| row[0] >= 19),
        "incomplete truth replay"
    );
    assert!(
        timings.calls > 0 && results[1][1] > 0,
        "person fixture did not run through the detector"
    );
}
