//! ffmpeg 子进程取流（T-326）：`ffmpeg -i <src> -vf fps=<n>,scale=1280:720 -pix_fmt bgr24 -f rawvideo -`
//! stdout 直读 BGR 帧——零绑定、RTSP/文件/USB（v4l2/dshow 由 ffmpeg 输入参数区分）全通用，
//! 与 Python 侧 deploy 部署的 ffmpeg.exe 同一来源。
//!
//! 重连：进程退出 → 指数退避（0.5s×2^n，上限 30s）重启；Stop 立即终止。

use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 可被控制线程共享的 ffmpeg 子进程句柄（stop 时强杀 → 读管道 EOF → 阻塞的
/// `read_frame` 立即返回，避免停止摄像头时数据面线程卡在管道上）。
pub type ChildSlot = Arc<Mutex<Option<Child>>>;

/// 取流配置（与 Python CameraWorker 语义对齐）。
#[derive(Clone, Debug)]
pub struct IngestConfig {
    pub source: String,
    /// 分析抽帧率（ffmpeg -vf fps=）
    pub analysis_fps: f32,
    /// 分析画布（Python 前端/区域坐标约定 1280x720）
    pub width: u32,
    pub height: u32,
    /// 文件源循环播放（LS_FILE_LOOP 语义）
    pub file_loop: bool,
    pub rtsp_transport_tcp: bool,
}

impl Default for IngestConfig {
    fn default() -> Self {
        IngestConfig {
            source: String::new(),
            analysis_fps: 5.0,
            width: 1280,
            height: 720,
            file_loop: false,
            rtsp_transport_tcp: true,
        }
    }
}

/// 一帧 BGR 图像（raw 像素，w*h*3）。
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    /// ffmpeg 管道内的帧序号（从 1 起）
    pub index: u64,
}

pub struct Ingest {
    cfg: IngestConfig,
    child: Option<ChildSlot>,
    stdout: Option<ChildStdout>,
    frame_index: u64,
    frame_bytes: usize,
}

impl Ingest {
    pub fn new(cfg: IngestConfig) -> Self {
        let frame_bytes = (cfg.width as usize) * (cfg.height as usize) * 3;
        Ingest {
            cfg,
            child: None,
            stdout: None,
            frame_index: 0,
            frame_bytes,
        }
    }

    fn build_cmd(&self) -> Command {
        let mut c = Command::new("ffmpeg");
        c.stdout(Stdio::piped()).stderr(Stdio::null());
        if self.cfg.rtsp_transport_tcp && self.cfg.source.starts_with("rtsp") {
            c.arg("-rtsp_transport").arg("tcp");
        }
        if self.cfg.file_loop {
            c.arg("-stream_loop").arg("-1");
        }
        c.arg("-i")
            .arg(&self.cfg.source)
            .args(["-an", "-sn", "-dn"]);
        if let Some(vf) = self.filter_chain() {
            c.args(["-vf", &vf]);
        }
        c.args(["-pix_fmt", "bgr24", "-f", "rawvideo", "pipe:1"]);
        c
    }

    /// 视频滤镜链：analysis_fps > 0 时 `fps,scale`（与 Python CameraWorker 语义一致，
    /// 由 ffmpeg 抽帧+缩放）；`analysis_fps == 0` 时**纯解码**（无 -vf：帧序/PTS/原始
    /// 分辨率都不经 swscale 重排，输出源分辨率——黄金语料逐位对齐模式，调用方自行缩放）。
    /// 返回 None = 不加 -vf。
    fn filter_chain(&self) -> Option<String> {
        if self.cfg.analysis_fps > 0.0 {
            Some(format!(
                "fps={},scale={}:{}",
                self.cfg.analysis_fps, self.cfg.width, self.cfg.height
            ))
        } else {
            None
        }
    }

    /// 启动（或断流后重启）ffmpeg；指数退避由调用方驱动 `spawn` 的 Err。
    pub fn spawn(&mut self) -> std::io::Result<()> {
        self.spawn_with_slot(Arc::new(Mutex::new(None)))
    }

    /// 用外部持有的 slot 启动：控制线程可 `Ingest::kill_slot(&slot)` 强杀。
    pub fn spawn_with_slot(&mut self, slot: ChildSlot) -> std::io::Result<()> {
        Ingest::kill_slot(&slot); // 清理上次退出的进程
        let mut c = self.build_cmd().spawn()?;
        self.stdout = c.stdout.take();
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(c);
        }
        self.child = Some(slot);
        self.frame_index = 0;
        Ok(())
    }

    /// 强杀 slot 里的 ffmpeg（可从任意线程调用；已空则 no-op）。
    pub fn kill_slot(slot: &ChildSlot) {
        if let Ok(mut guard) = slot.lock() {
            if let Some(mut c) = guard.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    /// 读一帧；Ok(None)=流结束（ffmpeg 退出），Err=IO 错误。调用方决定重连。
    pub fn read_frame(&mut self) -> std::io::Result<Option<Frame>> {
        if self.stdout.is_none() {
            return Ok(None);
        }
        let n = self.frame_bytes;
        let mut data = vec![0u8; n];
        let mut got = 0usize;
        let handle = self.stdout.as_mut().unwrap();
        while got < n {
            match handle.read(&mut data[got..]) {
                Ok(0) => break, // EOF
                Ok(k) => got += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if got < n {
            return Ok(None); // 流结束
        }
        self.frame_index += 1;
        Ok(Some(Frame {
            width: self.cfg.width,
            height: self.cfg.height,
            data,
            index: self.frame_index,
        }))
    }

    /// 进程是否仍在运行
    pub fn alive(&mut self) -> bool {
        self.child.as_ref().is_some_and(|slot| {
            slot.lock()
                .map(|mut g| g.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))))
                .unwrap_or(false)
        })
    }

    pub fn stop(&mut self) {
        self.stdout = None;
        if let Some(slot) = self.child.take() {
            Ingest::kill_slot(&slot);
        }
    }

    /// 重连退避：0.5s×2^n，上限 30s（n = 连续失败次数）
    pub fn backoff(attempt: u32) -> Duration {
        let s = 0.5f64 * 2f64.powi(attempt.min(6) as i32);
        Duration::from_secs_f64(s.min(30.0))
    }
}

impl Drop for Ingest {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机 ffmpeg 可用时才跑（CI 干净环境 skip）。
    fn ffmpeg_available() -> bool {
        Command::new("ffmpeg").arg("-version").output().is_ok()
    }

    #[test]
    fn reads_synthetic_video_frames() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg 不可用");
            return;
        }
        // 合成 2s 10fps 测试视频（ffmpeg testsrc）到临时文件
        let dir = std::env::temp_dir().join("ls-ingest-test");
        std::fs::create_dir_all(&dir).unwrap();
        let v = dir.join("t.mp4");
        let st = Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=2:size=320x240:rate=10",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&v)
            .output()
            .unwrap();
        assert!(st.status.success());
        let mut ing = Ingest::new(IngestConfig {
            source: v.to_string_lossy().to_string(),
            analysis_fps: 5.0,
            width: 320,
            height: 240,
            ..Default::default()
        });
        ing.spawn().unwrap();
        let mut frames = 0;
        while let Some(f) = ing.read_frame().unwrap() {
            assert_eq!(f.data.len(), 320 * 240 * 3);
            frames += 1;
        }
        ing.stop();
        assert!(
            (8..=12).contains(&frames),
            "2s@5fps 应抽 ~10 帧，实得 {frames}"
        );
        let _ = std::fs::remove_file(&v);
    }
}
