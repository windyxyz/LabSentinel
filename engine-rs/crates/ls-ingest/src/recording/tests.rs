use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;

use super::*;
use crate::capture::FrameReader;
use crate::{ChildSlot, IngestConfig};

fn arrival() -> Arrival {
    Arrival {
        wall: SystemTime::now(),
        monotonic: Instant::now(),
    }
}

fn after(base: Arrival, elapsed: Duration) -> Arrival {
    Arrival {
        wall: base.wall + elapsed,
        monotonic: base.monotonic + elapsed,
    }
}

#[test]
fn cold_start_resets_on_discontinuity_and_releases_request_slots() {
    let recorder = Recorder::new();
    let base = arrival();
    let push = |secs, keyframe| {
        recorder.0.state.lock().unwrap().push(
            Fragment::synthetic(secs as i64 * TIMESCALE, TIMESCALE, keyframe, 32),
            &recorder.0.bytes,
            after(base, Duration::from_secs(secs)),
        );
    };
    let trigger = |secs| {
        recorder.trigger(
            base.wall + Duration::from_secs(secs),
            Duration::ZERO,
            Duration::from_secs(2),
        )
    };
    push(0, false);
    assert_eq!(trigger(0).err(), Some(RecordingError::NoKeyframe));
    push(1, true);
    let pending: Vec<_> = (0..MAX_PENDING_CLIPS)
        .map(|_| trigger(1).unwrap())
        .collect();
    assert_eq!(trigger(1).err(), Some(RecordingError::Busy));
    push(12, false);
    assert_eq!(
        trigger(12).err(),
        Some(RecordingError::TimestampDiscontinuity)
    );
    push(13, true);
    for old in pending {
        assert_eq!(
            old.wait(Duration::ZERO).err(),
            Some(RecordingError::TimestampDiscontinuity)
        );
    }
    assert!(trigger(13).is_ok());
}

#[test]
fn evicts_whole_gops_under_byte_pressure_including_pinned_fragments() {
    let recorder = Recorder::new();
    let mut state = recorder.0.state.lock().unwrap();
    let base = arrival();
    let push = |state: &mut State, frame: i64, keyframe| {
        state.push(
            Fragment::synthetic(frame * 100, 100, keyframe, 3 * 1024 * 1024),
            &recorder.0.bytes,
            base,
        );
        assert!(recorder.0.bytes.load(Ordering::Relaxed) <= BUFFER_BYTES);
        assert!(state.groups.iter().all(|group| group[0].sample.keyframe));
    };
    for f in 0..4 {
        push(&mut state, f, f == 0);
    }
    let pinned = state.groups[0].clone();
    for f in 4..8 {
        push(&mut state, f, f == 4);
    }
    assert!(state.groups.is_empty(), "discard the incomplete second GOP");
    assert_eq!(state.count, 0);
    assert!(recorder.0.bytes.load(Ordering::Relaxed) >= 12 * 1024 * 1024);
    drop(pinned);
    assert_eq!(recorder.0.bytes.load(Ordering::Relaxed), 0);
    push(&mut state, 8, true);
    assert_eq!(state.count, 1);
}

#[test]
fn bounds_fragment_count_and_source_time() {
    let recorder = Recorder::new();
    let mut state = recorder.0.state.lock().unwrap();
    let base = arrival();
    let budget = &recorder.0.bytes;
    for index in 0i64..1500 {
        state.push(
            Fragment::synthetic(index * 100, 100, index % 100 == 0, 32),
            budget,
            base,
        );
    }
    assert!(state.count <= MAX_FRAGMENTS);
    assert_eq!(
        state.count,
        state.groups.iter().map(Vec::len).sum::<usize>()
    );
    assert!(state.groups[0][0].sample.keyframe);
    state.reset(RecordingError::TimestampDiscontinuity);
    for secs in 0..21 {
        state.push(
            Fragment::synthetic(secs as i64 * TIMESCALE, TIMESCALE, secs % 2 == 0, 32),
            budget,
            after(base, Duration::from_secs(secs)),
        );
    }
    assert_eq!(state.groups[0][0].sample.dts, 8 * TIMESCALE);
}

#[test]
fn initial_probe_burst_is_aligned_to_arrival_and_wall_clock_steps_break_the_epoch() {
    let base = arrival();
    let clock = SourceClock {
        anchor: base.wall,
        first_dts: 0,
        first_pts: 0,
        previous: base,
    };
    let newest = after(base, Duration::from_millis(100));
    let clock = clock.observe(4 * TIMESCALE, newest).unwrap();
    assert_eq!(clock.at(4 * TIMESCALE), Some(newest.wall));
    let next = Arrival {
        wall: newest.wall + Duration::from_secs(2),
        monotonic: newest.monotonic + Duration::from_millis(100),
    };
    assert!(clock
        .observe(4 * TIMESCALE + TIMESCALE / 10, next)
        .is_none());
}

fn ffmpeg() -> Command {
    let mut cmd = media_command("ffmpeg");
    cmd.args(["-hide_banner", "-v", "error", "-nostdin"]);
    cmd
}

fn media_command(name: &str) -> Command {
    let path = ls_core::bin::find(name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new(path);
        cmd.creation_flags(0x0800_0000);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(path)
    }
}

fn run(cmd: &mut Command) -> Output {
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn synthetic_source(dir: &Path, extension: &str, encoder: &str, options: &[&str]) -> PathBuf {
    let output = dir.join(format!("{encoder}.{extension}"));
    run(ffmpeg()
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=4:size=96x64:rate=20",
            "-c:v",
            encoder,
            "-threads",
            "1",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(options)
        .arg(&output));
    output
}

fn capture_file(path: &Path, slot: &ChildSlot) -> Recorder {
    let (mut reader, recorder) = FrameReader::start_with_recording(
        IngestConfig {
            source: path.to_string_lossy().into(),
            width: 48,
            height: 32,
            analysis_fps: 5.0,
            ..IngestConfig::default()
        },
        Arc::clone(slot),
        false,
        false,
    )
    .unwrap();
    let mut decoded = 0;
    while let Some(frame) = reader.read().unwrap() {
        assert_eq!(frame.frame.data.len(), 48 * 32 * 3);
        decoded += 1;
    }
    assert_eq!(decoded, 20);
    assert!(reader.file_exit_ok());
    drop(reader);
    recorder.unwrap()
}

fn source_hashes(path: &Path, codec: VideoCodec) -> Vec<String> {
    let result = run(ffmpeg().arg("-i").arg(path).args([
        "-map",
        "0:v:0",
        "-c:v",
        "copy",
        "-f",
        "framehash",
        "pipe:1",
    ]));
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(
        text.contains(&format!("#codec_id 0: {}", codec.as_str())),
        "{text}"
    );
    assert!(text.contains("#dimensions 0: 96x64"), "{text}");
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.rsplit(',').next().unwrap().trim().to_string())
        .collect()
}

fn exercise_real_clip(dir: &Path, source: &Path, codec: VideoCodec) {
    let slot = ChildSlot::default();
    let recorder = capture_file(source, &slot);
    assert_eq!(
        fs::read_dir(dir).unwrap().count(),
        1,
        "idle ingestion wrote a file"
    );
    let state = recorder.0.state.lock().unwrap();
    let anchor = state.presentation_time(&state.groups[0][0]).unwrap();
    drop(state);
    let event = anchor + Duration::from_millis(2250);
    let pending = recorder
        .trigger(
            event,
            Duration::from_millis(700),
            Duration::from_millis(650),
        )
        .unwrap();
    let clip = pending.wait(Duration::ZERO).unwrap();
    let output = dir.join("事件片段.mp4");
    let coverage = clip.write_mp4(&output).unwrap();
    assert_eq!(coverage.codec, codec);
    if ls_core::bin::available("ffprobe") {
        let probe = run(media_command("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,codec_tag_string",
                "-of",
                "csv=p=0",
            ])
            .arg(&output));
        let expected = match codec {
            VideoCodec::H264 => "h264,avc1",
            VideoCodec::Hevc => "hevc,hvc1",
        };
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), expected);
    }
    assert_eq!(coverage.truncation, None);
    assert!(coverage.start > anchor);
    assert!(coverage.start <= event - Duration::from_millis(700));
    assert!(coverage.end >= event + Duration::from_millis(650));
    let file = fs::read(&output).unwrap();
    assert!(coverage.keyframes.len() >= 2);
    assert_eq!(coverage.keyframes[0].timestamp, coverage.start);
    for keyframe in coverage.keyframes {
        let offset = keyframe.offset as usize;
        assert_eq!(&file[offset + 4..offset + 8], b"moof");
    }
    assert_eq!(
        fs::read_dir(dir).unwrap().count(),
        2,
        "temporary file was retained"
    );
    run(ffmpeg()
        .args(["-xerror", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"]));
    let source_hashes = source_hashes(source, codec);
    let clip_hashes = self::source_hashes(&output, codec);
    assert!(clip_hashes.len() < source_hashes.len());
    assert!(
        source_hashes
            .windows(clip_hashes.len())
            .any(|original| original == clip_hashes),
        "video packets were changed or reordered"
    );

    // A late event retains the valid part of its old session after reconnect.
    let late_event = anchor + Duration::from_millis(3500);
    let pending = recorder
        .trigger(
            late_event,
            Duration::from_millis(500),
            Duration::from_secs(2),
        )
        .unwrap();
    let restarted = capture_file(source, &slot);
    assert!(!Arc::ptr_eq(&recorder.0, &restarted.0));
    let disconnected = pending.wait(Duration::from_secs(1)).unwrap();
    let coverage = disconnected
        .write_mp4(&dir.join("断开时的事件.mp4"))
        .unwrap();
    assert_eq!(coverage.truncation, Some(RecordingError::SourceEnded));
    assert!(coverage.start <= late_event && late_event <= coverage.end);
}

#[test]
fn captures_and_saves_original_h264_packets_with_decoding_active() {
    if !ls_core::bin::available("ffmpeg") {
        eprintln!("skip: ffmpeg unavailable");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = synthetic_source(
        dir.path(),
        "mp4",
        "libx264",
        &[
            "-g",
            "20",
            "-keyint_min",
            "20",
            "-sc_threshold",
            "0",
            "-bf",
            "2",
        ],
    );
    exercise_real_clip(dir.path(), &source, VideoCodec::H264);
}

#[test]
fn captures_and_saves_original_hevc_packets_with_decoding_active() {
    if !ls_core::bin::available("ffmpeg") {
        eprintln!("skip: ffmpeg unavailable");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = synthetic_source(
        dir.path(),
        "mp4",
        "libx265",
        &[
            "-x265-params",
            "keyint=20:min-keyint=20:scenecut=0:open-gop=0:frame-threads=1:pools=1",
        ],
    );
    exercise_real_clip(dir.path(), &source, VideoCodec::Hevc);
}

#[test]
fn unsupported_recording_keeps_the_decoded_output_available() {
    if !ls_core::bin::available("ffmpeg") {
        eprintln!("skip: ffmpeg unavailable");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for encoder in ["ffv1", "mpeg4"] {
        let source = synthetic_source(dir.path(), "mkv", encoder, &[]);
        assert_eq!(
            capture_file(&source, &ChildSlot::default())
                .trigger(SystemTime::now(), Duration::ZERO, Duration::ZERO,)
                .err(),
            Some(RecordingError::Unavailable)
        );
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn live_trigger_waits_for_post_samples_without_blocking_decoded_frames() {
    if !ls_core::bin::available("ffmpeg") {
        eprintln!("skip: ffmpeg unavailable");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = synthetic_source(
        dir.path(),
        "ts",
        "libx264",
        &[
            "-g",
            "20",
            "-keyint_min",
            "20",
            "-sc_threshold",
            "0",
            "-bf",
            "2",
        ],
    );
    let mut cmd = ffmpeg();
    cmd.args(["-loglevel", "quiet", "-nostats", "-re", "-i"])
        .arg(&source)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            "fps=5,scale=48:32",
            "-pix_fmt",
            "bgr24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    add_ffmpeg_output(&mut cmd);
    let mut producer = cmd.spawn().unwrap();
    let (recorder, recorder_thread) = Recorder::start(producer.stderr.take().unwrap()).unwrap();
    let mut raw = producer.stdout.take().unwrap();
    let (send, received) = mpsc::channel();
    let consumer = thread::spawn(move || {
        let mut frames = 0;
        let mut buffer = [0u8; 48 * 32 * 3];
        while raw.read_exact(&mut buffer).is_ok() {
            frames += 1;
            if frames == 8 {
                let _ = send.send(SystemTime::now());
            }
        }
        frames
    });
    let event = received.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    let pre = Duration::from_millis(600);
    let post = Duration::from_millis(550);
    let pending = recorder.trigger(event, pre, post).unwrap();
    let clip = pending.wait(Duration::from_secs(3)).unwrap();
    let output = dir.path().join("live.mp4");
    let coverage = clip.write_mp4(&output).unwrap();
    assert_eq!(coverage.truncation, None);
    assert!(coverage.start <= event - pre);
    assert!(coverage.end >= event + post);
    assert!(producer.wait().unwrap().success());
    assert_eq!(consumer.join().unwrap(), 20);
    recorder_thread.join().unwrap();
    run(ffmpeg()
        .args(["-xerror", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"]));
}

/// Local measurement only. Filesystem and VM load affect write/rename latency.
#[test]
#[ignore = "measure on the Windows acceptance host"]
fn materializes_ten_seconds_of_720p_video_on_demand() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    run(ffmpeg()
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=12:size=1280x720:rate=25",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-threads",
            "4",
            "-b:v",
            "2M",
            "-g",
            "25",
            "-sc_threshold",
            "0",
        ])
        .arg(&source));
    let mut cmd = ffmpeg();
    cmd.args(["-loglevel", "quiet", "-nostats", "-i"])
        .arg(&source)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    add_ffmpeg_output(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    let (recorder, thread) = Recorder::start(child.stderr.take().unwrap()).unwrap();
    assert!(child.wait().unwrap().success());
    thread.join().unwrap();
    let state = recorder.0.state.lock().unwrap();
    let anchor = state.presentation_time(&state.groups[0][0]).unwrap();
    drop(state);
    let event = anchor + Duration::from_secs(6);
    let output = dir.path().join("event.mp4");
    let start = Instant::now();
    let coverage = recorder
        .trigger(event, Duration::from_secs(5), Duration::from_secs(5))
        .unwrap()
        .wait(Duration::ZERO)
        .unwrap()
        .write_mp4(&output)
        .unwrap();
    let elapsed = start.elapsed();
    let duration = coverage.end.duration_since(coverage.start).unwrap();
    assert!((Duration::from_secs(10)..=Duration::from_millis(10_100)).contains(&duration));
    assert_eq!(coverage.truncation, None);
    eprintln!(
        "fMP4 {duration:?}, {} bytes, selection + publish {elapsed:?}",
        fs::metadata(&output).unwrap().len()
    );
    run(ffmpeg()
        .args(["-xerror", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"]));
}
