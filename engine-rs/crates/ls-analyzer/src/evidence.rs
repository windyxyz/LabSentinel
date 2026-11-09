//! T-326：事件存证——证据环形缓冲（JPEG@evidence_fps 抽帧）+ 事件前后片段写盘。
//!
//! 口径与 Python `CameraWorker._feed_recordings`/`_write_video` 对齐（T-308）：
//! - JPEG 质量 80，抽帧率 `LS_EVIDENCE_FPS`（默认 10），stride = round(in_fps/efps)；
//! - 环形缓冲容量 = int(min(efps, in_fps) × pre_s) + 10（源低于抽帧率按源计）；
//! - 事件后由数据面继续喂 post_s 秒帧（`ActiveRec`），RPC 处理器等待补齐后
//!   用 ffmpeg 写 MP4（libx264/yuv420p 优先——浏览器可播，mpeg4 兜底）。
//! - 与 Python 的差异（有意）：输出帧率取**实际喂帧率**（Python `_rec_fps` 用
//!   源帧率而帧只有抽帧率 → 慢放 bug，Python 侧另行修复）。

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const VIDEO_PRE_S: f64 = 2.0;
pub const VIDEO_POST_S: f64 = 2.0;
/// 与 Python MAX_CONCURRENT_RECORDINGS 一致：超限退化（本实现=拒绝并回执失败）
pub const MAX_CONCURRENT_RECORDINGS: usize = 3;

#[derive(Clone)]
pub struct EvidenceFrame {
    /// 源帧序号（诊断/测试断言用；写盘只消费 data）
    #[allow(dead_code)]
    pub index: u64,
    pub data: Vec<u8>,
}

/// 每路证据环形缓冲（数据面单线程写、RPC 线程读——Mutex 足够）。
pub struct EvidenceRing {
    q: VecDeque<EvidenceFrame>,
    stride_cnt: u64,
    stride: u64,
    cap: usize,
    /// 写盘帧率 = 实际喂帧率 min(efps, in_fps)（视频播放速度正确）
    out_fps: f64,
}

impl EvidenceRing {
    pub fn new(in_fps: f64, efps: f64) -> Self {
        let efps = efps.max(1.0);
        let in_fps = in_fps.max(0.1);
        let stride = ((in_fps / efps).round() as u64).max(1);
        let cap_fps = efps.min(in_fps);
        EvidenceRing {
            q: VecDeque::new(),
            stride_cnt: 0,
            stride,
            cap: (cap_fps * VIDEO_PRE_S) as usize + 10,
            out_fps: cap_fps,
        }
    }

    /// 本帧是否命中抽帧（逐帧调用；与 Python _ring_count 步进口径一致）
    pub fn hit(&mut self) -> bool {
        let h = self.stride_cnt.is_multiple_of(self.stride);
        self.stride_cnt += 1;
        h
    }

    pub fn push(&mut self, index: u64, jpeg: Vec<u8>) {
        while self.q.len() >= self.cap {
            self.q.pop_front();
        }
        self.q.push_back(EvidenceFrame { index, data: jpeg });
    }

    /// 全量快照（≈pre_s 秒；事件帧应已在队尾）
    pub fn snapshot(&self) -> Vec<EvidenceFrame> {
        self.q.iter().cloned().collect()
    }

    pub fn latest(&self) -> Option<&EvidenceFrame> {
        self.q.back()
    }

    pub fn out_fps(&self) -> f64 {
        self.out_fps
    }
}

/// 一次事件录像：pre 帧由 RPC 处理器从环形缓冲拷出，post 帧由数据面补齐。
pub struct ActiveRec {
    /// Mutex：测试可回拨起点（Arc 共享下 DerefMut 不可用）
    pub start: Mutex<Instant>,
    pub frames: Mutex<Vec<Vec<u8>>>,
    pub done: AtomicBool,
}

impl ActiveRec {
    pub fn new(pre: Vec<Vec<u8>>) -> Self {
        ActiveRec {
            start: Mutex::new(Instant::now()),
            frames: Mutex::new(pre),
            done: AtomicBool::new(false),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }
}

/// 每路共享存证状态（数据面写 / RPC 处理器读）。
#[derive(Default)]
pub struct Evidence {
    pub ring: Mutex<Option<EvidenceRing>>, // None=未初始化（旧客户端兼容）
    pub recs: Mutex<Vec<std::sync::Arc<ActiveRec>>>,
}

impl Evidence {
    pub fn new_ring(&self, in_fps: f64, efps: f64) {
        *self.ring.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(EvidenceRing::new(in_fps, efps));
    }

    /// 数据面逐帧喂入：命中抽帧则编码入环并补进活动录像；录像到期置 done。
    /// 返回本帧 JPEG（仅命中抽帧时；事件帧快照由调用方直接编码，
    /// 录像只消费 stride 帧，与 Python `_feed_recordings` 同口径）。
    pub fn feed(&self, frame_index: u64, encode: impl FnOnce() -> Vec<u8>) -> Option<Vec<u8>> {
        // 收尾检查每帧都跑（done 时刻比 Python 的 stride 量化更精确，无害）
        {
            let recs = self.recs.lock().unwrap_or_else(|e| e.into_inner());
            let post_window = Duration::from_secs_f64(VIDEO_POST_S + 0.5);
            for rec in recs.iter() {
                if !rec.done.load(Ordering::SeqCst) && rec.elapsed() >= post_window {
                    rec.done.store(true, Ordering::SeqCst);
                }
            }
        }
        let mut ring_guard = self.ring.lock().unwrap_or_else(|e| e.into_inner());
        let ring = ring_guard.as_mut()?;
        if !ring.hit() {
            return None;
        }
        let jpeg = encode();
        ring.push(frame_index, jpeg.clone());
        let recs = self.recs.lock().unwrap_or_else(|e| e.into_inner());
        for rec in recs.iter() {
            if !rec.done.load(Ordering::SeqCst) {
                rec.frames
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(jpeg.clone());
            }
        }
        Some(jpeg)
    }

    /// 数据面退出时收尾：未补齐的录像立即置 done（帧数不足也落盘，有比没有强）。
    pub fn flush(&self) {
        for rec in self.recs.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            rec.done.store(true, Ordering::SeqCst);
        }
    }
}

/// JPEG 编码（BGR→RGB 交换 + jpeg-encoder 质量 80，与 Python imencode 同参）。
pub fn encode_jpeg_bgr(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut rgb = data.to_vec();
    for px in rgb.as_chunks_mut::<3>().0 {
        px.swap(0, 2);
    }
    let mut out = Vec::with_capacity(rgb.len() / 4);
    let enc = jpeg_encoder::Encoder::new(&mut out, 80);
    enc.encode(
        &rgb,
        width as u16,
        height as u16,
        jpeg_encoder::ColorType::Rgb,
    )
    .map_err(|e| format!("jpeg encode: {e}"))?;
    Ok(out)
}

/// 把 JPEG 帧序列写为 MP4（libx264/yuv420p 优先，mpeg4 兜底）。
/// 返回 Ok(()) 或两档都失败的错误串。
pub fn encode_mp4(frames: &[Vec<u8>], fps: f64, out: &Path) -> Result<(), String> {
    if frames.is_empty() {
        return Err("no frames".into());
    }
    let codecs: [&[&str]; 2] = [
        &[
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ],
        &["-c:v", "mpeg4", "-q:v", "5"],
    ];
    let mut last_err = String::new();
    for codec in codecs {
        match encode_mp4_with(frames, fps, out, codec) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

fn encode_mp4_with(frames: &[Vec<u8>], fps: f64, out: &Path, codec: &[&str]) -> Result<(), String> {
    // R2-301/R2-302: portable ffmpeg lookup + drain stderr in a thread so
    // verbose ffmpeg output cannot fill the pipe and deadlock the child.
    let mut child = std::process::Command::new(ls_core::bin::find("ffmpeg"))
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "image2pipe",
            "-framerate",
            &format!("{fps:.3}"),
            "-i",
            "pipe:0",
        ])
        .args(codec)
        .args(["-movflags", "+faststart", "-y", &out.to_string_lossy()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg spawn: {e}"))?;
    let stderr = child.stderr.take().ok_or("ffmpeg stderr 不可用")?;
    let err_thread = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut r = stderr;
        let _ = r.read_to_end(&mut buf);
        buf
    });
    {
        let mut stdin = child.stdin.take().ok_or("ffmpeg stdin 不可用")?;
        for f in frames {
            stdin.write_all(f).map_err(|e| format!("pipe write: {e}"))?;
        }
    }
    let res = child.wait().map_err(|e| format!("ffmpeg wait: {e}"))?;
    let err_bytes = err_thread.join().unwrap_or_default();
    if res.success() && out.is_file() {
        return Ok(());
    }
    let tail = String::from_utf8_lossy(&err_bytes);
    let detail = tail
        .trim()
        .chars()
        .rev()
        .take(500)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    Err(format!("ffmpeg {codec:?} 失败: {res:?}; stderr: {detail}"))
}

/// 视频目录相对 data_dir 的 relpath（proto：video_relpath 相对 data_dir）。
pub fn video_relpath(video_dir: &Path, data_dir: Option<&Path>, file: &str) -> String {
    let base = match data_dir {
        Some(d) if video_dir.strip_prefix(d).is_ok() => {
            video_dir.strip_prefix(d).unwrap_or(video_dir)
        }
        _ => video_dir,
    };
    let mut p = base.to_path_buf();
    p.push(file);
    p.to_string_lossy().replace('\\', "/")
}

/// 环形单元测试：stride/容量/snapshot 语义。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_stride_and_cap() {
        // 30fps 源、10fps 抽帧：stride=3、out_fps=10、cap=10*2+10=30
        let mut r = EvidenceRing::new(30.0, 10.0);
        assert_eq!(r.out_fps(), 10.0);
        let mut pushed = 0;
        for i in 0..90u64 {
            if r.hit() {
                r.push(i, vec![i as u8]);
                pushed += 1;
            }
        }
        assert_eq!(pushed, 30); // 90 帧 ÷ stride3
        assert_eq!(r.snapshot().len(), 30); // 容量截断
        assert_eq!(r.latest().unwrap().index, 87); // 命中帧 = 0,3,...,87
                                                   // 低帧率源：in_fps < efps → stride=1、cap 按源计
        let mut r = EvidenceRing::new(5.0, 10.0);
        assert_eq!(r.out_fps(), 5.0);
        for i in 0..25u64 {
            if r.hit() {
                r.push(i, vec![0]);
            }
        }
        assert_eq!(r.snapshot().len(), 20); // 5*2+10
    }

    #[test]
    fn feed_records_done() {
        let ev = Evidence::default();
        ev.new_ring(5.0, 10.0);
        let rec = std::sync::Arc::new(ActiveRec::new(vec![vec![1], vec![2]]));
        ev.recs.lock().unwrap().push(std::sync::Arc::clone(&rec));
        // in_fps=5 → stride=1 → 每帧命中；首帧未到期
        let j = ev.feed(1, || vec![3]);
        assert!(j.is_some());
        assert_eq!(rec.frames.lock().unwrap().len(), 3);
        assert!(!rec.done.load(Ordering::SeqCst));
        // 回拨 start 模拟到期（post 窗口 2.5s）
        *rec.start.lock().unwrap() = Instant::now() - Duration::from_secs_f64(3.0);
        ev.feed(2, || vec![4]);
        assert!(rec.done.load(Ordering::SeqCst));
    }

    #[test]
    fn relpath_handling() {
        assert_eq!(
            video_relpath(
                Path::new("D:/data/videos"),
                Some(Path::new("D:/data")),
                "ev_1.mp4"
            ),
            "videos/ev_1.mp4"
        );
        assert_eq!(
            video_relpath(Path::new("D:/vids"), None, "ev_1.mp4"),
            "D:/vids/ev_1.mp4"
        );
    }

    #[test]
    fn encode_mp4_integration() {
        // R2-302: portable lookup; ffmpeg 可用时**必须**通过（不再静默吞失败）。
        if !ls_core::bin::available("ffmpeg") {
            eprintln!("skip: ffmpeg 不可用");
            return;
        }
        let (w, h) = (64u32, 48u32);
        let mut frames = vec![];
        for i in 0..20u32 {
            let mut bgr = Vec::with_capacity((w * h * 3) as usize);
            for y in 0..h {
                for x in 0..w {
                    bgr.push((x * 4 % 256) as u8);
                    bgr.push((y * 4 % 256) as u8);
                    bgr.push((i * 12 % 256) as u8);
                }
            }
            frames.push(encode_jpeg_bgr(&bgr, w, h).unwrap());
        }
        let dir = std::env::temp_dir().join("ls-ev-test");
        std::fs::create_dir_all(&dir).unwrap();
        let out: std::path::PathBuf = dir.join("ev_test.mp4");
        encode_mp4(&frames, 10.0, &out).expect("encode");
        let meta = std::fs::metadata(&out).unwrap();
        assert!(meta.len() > 500, "mp4 过小: {meta:?}");
        let _ = std::fs::remove_file(&out);
    }
}
