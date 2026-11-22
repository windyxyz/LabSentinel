//! ffmpeg 子进程取流（T-326）：`ffmpeg -i <src> -vf fps=<n>,scale=1280:720 -pix_fmt bgr24 -f rawvideo -`
//! stdout 直读 BGR 帧——零绑定、RTSP/文件/USB（v4l2/dshow 由 ffmpeg 输入参数区分）全通用，
//! 与 Python 侧 deploy 部署的 ffmpeg.exe 同一来源。
//!
//! 重连：进程退出 → 指数退避（0.5s×2^n，上限 30s）重启；Stop 立即终止。

/// T-326：图片文件夹源的候选扫描器（纯函数，与 Python `folders._candidates` 同口径）。
pub mod folder;

// 2026-11-21（用户实测故障根治）：ffmpeg 孤儿治理——Job Object kill-on-close，
// server 以任何方式死亡时内核自动杀掉全部 ffmpeg（不再残留占摄像头）。
mod winjob;

/// LS-051：Rust 托管的外部子进程守护（go2rtc 等）——绑 Windows Job Object
/// kill-on-close，主进程任何时候死亡都不留孤儿；Drop 自动杀进程。
/// 复用 winjob 内部已收口的 unsafe，本模块零新增 unsafe。
pub mod child_guard;

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
    /// 孤儿治理（2026-11-21）：所有 ffmpeg 子进程 assign 进此 job——server 进程
    /// 以任何方式死亡（崩溃/taskkill/任务管理器）时句柄关闭，内核杀掉 job 内
    /// 全部 ffmpeg。None = job 创建失败（降级为旧行为）。
    job: Option<crate::winjob::JobHandle>,
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
            job: crate::winjob::JobHandle::new(),
        }
    }

    fn build_cmd(&self) -> Command {
        // R2-301/N-22: portable layout keeps ffmpeg in tools/ (not on PATH).
        let mut c = Command::new(ls_core::bin::find("ffmpeg"));
        c.stdout(Stdio::piped()).stderr(Stdio::null());
        // LS-043：GUI 子系统下禁止弹 cmd 黑框
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        if self.cfg.source.starts_with("rtsp") {
            if self.cfg.rtsp_transport_tcp {
                c.arg("-rtsp_transport").arg("tcp");
            }
            // LS-043：RTSP 读写超时 10s，防止网络拔线半开连接永久阻塞 read_frame
            c.args(["-rw_timeout", "10000000"]);
        }
        if self.cfg.file_loop {
            c.arg("-stream_loop").arg("-1");
        }
        // USB(dshow)：source 形如 `video=<设备名>`（list_usb_devices 产出的标准
        // dshow 语法；前端手填由 usb_devices handler/前端归一化）。裸 `/dev/video*`
        // (linux v4l2) 与 RTSP/文件路径不受影响。
        if self.cfg.source.starts_with("video=") {
            c.arg("-f").arg("dshow");
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
        // 孤儿治理：子进程挂进 server 生命期的 job（kill-on-close）——assign
        // 失败（进程秒退等）无害，正常 kill_slot 路径不受影响
        if let Some(job) = &self.job {
            let _ = job.assign_child(&c);
        }
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

/// RA2-S3：单图解码为 BGR 画布（folder worker 识别管线）。
/// `ffmpeg -i <img> -frames:v 1 -vf scale=WxH -pix_fmt bgr24 -f rawvideo pipe:1`
/// ——零新依赖、解码器与视频路完全一致、天然支持扫描器全部白名单扩展名；
/// 中文路径安全（Command 参数不经过 shell）。返回 `(bgr, width, height)`；
/// 解码失败/缺 ffmpeg/字节数不符 → Err（调用方落库 error 字段如实）。
pub fn decode_image(
    path: &std::path::Path,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, u32, u32), String> {
    let mut c = Command::new(ls_core::bin::find("ffmpeg"));
    c.args(["-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={width}:{height}"),
            "-pix_fmt",
            "bgr24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.spawn().map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout
            .read_to_end(&mut out)
            .map_err(|e| format!("读帧失败: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("ffmpeg 等待失败: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg 退出码 {}", status));
    }
    let expect = (width as usize) * (height as usize) * 3;
    if out.len() != expect {
        return Err(format!("帧字节数 {} != 期望 {expect}", out.len()));
    }
    Ok((out, width, height))
}

/// USB(dshow) 摄像头（2026-11-21，用户实测反馈"手动填索引太麻烦"）：
/// `ffmpeg -list_devices true -f dshow -i dummy` 枚举本机摄像头（发行包自带
/// ffmpeg，零新依赖）。返回 `(索引, 设备名, source)`——source 为 ffmpeg dshow
/// 原生语法 `video=<设备名>`（取流直通 build_cmd 的 `-f dshow` 分支）。
#[derive(Debug, Clone, PartialEq)]
pub struct UsbDevice {
    pub index: usize,
    pub name: String,
    pub source: String,
}

/// 解析 ffmpeg dshow 枚举输出（stderr）：
/// `[dshow @ 0000...] "HD Webcam" (video)`——只取 video 段（audio 行以 `(audio)`
/// 结尾；Alternative name 行无引号包裹形态）。UTF-8 解码（ffmpeg 5.x 起
/// Windows 控制台输出 UTF-8；非法字节 lossy 兜底，不崩）。
pub fn parse_dshow_devices(stderr: &str) -> Vec<UsbDevice> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        // 形态：[dshow @ xxx] "name" (video)——引号开头 + 行尾 (video)
        let Some(quoted_start) = line.find('"') else {
            continue;
        };
        let rest = &line[quoted_start + 1..];
        let Some(quoted_end) = rest.find('"') else {
            continue;
        };
        let name = &rest[..quoted_end];
        let tail = rest[quoted_end + 1..].trim();
        if tail != "(video)" || name.is_empty() {
            continue;
        }
        let index = out.len();
        out.push(UsbDevice {
            index,
            name: name.to_string(),
            source: format!("video={name}"),
        });
    }
    out
}

/// 枚举本机 USB 摄像头（Windows dshow；非 Windows 返回 Err——v4l2 场景手填
/// `/dev/video0` 直通）。失败文案保留前端回退手输的可用性。
pub fn list_usb_devices() -> Result<Vec<UsbDevice>, String> {
    #[cfg(not(windows))]
    {
        Err("USB 设备枚举仅支持 Windows（dshow）；Linux 请手填 /dev/video*".into())
    }
    #[cfg(windows)]
    {
        use std::io::Read;
        let mut c = Command::new(ls_core::bin::find("ffmpeg"));
        c.args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
        // CREATE_NO_WINDOW：GUI 子系统服务进程调用时不闪控制台框
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
        }
        let mut child = c.spawn().map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
        let err_pipe = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = err_pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });
        // LS-043：真实 15s 超时保护，防止损坏的 DirectShow 驱动永久挂死 HTTP 线程
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        let stderr = reader.join().unwrap_or_default();
        let devices = parse_dshow_devices(&String::from_utf8_lossy(&stderr));
        if devices.is_empty() {
            return Err("未检测到 USB 摄像头（dshow 枚举为空）".into());
        }
        Ok(devices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机 ffmpeg 可用时才跑（CI 干净环境 skip）。
    fn ffmpeg_available() -> bool {
        ls_core::bin::available("ffmpeg")
    }

    #[test]
    fn dshow_parse_video_only_and_index_order() {
        // 实测输出形态（ffmpeg 7.x，中文 Windows UTF-8）：video/audio 混排 +
        // Alternative name 行 + 多设备
        let sample = "[dshow @ 0001] DirectShow video devices (some may be both):
[dshow @ 0001] \"HD Webcam\" (video)
[dshow @ 0001]   Alternative name \"@device_pnp_\\?\\usb#vid_2b7e&pid_b663#global\"
[dshow @ 0001] \"虚拟摄像机\" (video)
[dshow @ 0001] DirectShow audio devices:
[dshow @ 0001] \"麦克风阵列 (Senary Audio)\" (audio)
";
        let d = parse_dshow_devices(sample);
        assert_eq!(d.len(), 2, "只取 video 段：{d:?}");
        assert_eq!(d[0].name, "HD Webcam");
        assert_eq!(d[0].source, "video=HD Webcam");
        assert_eq!(d[0].index, 0);
        assert_eq!(d[1].name, "虚拟摄像机", "中文设备名 UTF-8");
        assert_eq!(d[1].source, "video=虚拟摄像机");
        assert_eq!(d[1].index, 1);
    }

    #[test]
    fn dshow_parse_empty_and_malformed() {
        assert!(parse_dshow_devices("").is_empty());
        assert!(parse_dshow_devices("Error opening input file dummy.").is_empty());
        // 无 (video) 标记的引号行不算设备
        assert!(parse_dshow_devices("[dshow @ x] \"weird\" (unknown)").is_empty());
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
        let st = Command::new(ls_core::bin::find("ffmpeg"))
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
