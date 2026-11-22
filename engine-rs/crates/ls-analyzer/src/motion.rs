//! LS-050 性能底座：Motion Gate 轻量帧差预筛。
//!
//! 空场景跳过 YOLO/PPE 推理，算力直降。防漏判红线（三条同时满足才跳过）：
//!   1. 当前无活跃 track（有 track 必须全检——人已在场时绝不能靠帧差偷懒）；
//!   2. 当前帧与上一帧的下采样灰度平均绝对差 < 阈值；
//!   3. 距上次全检未超过心跳间隔（默认每 10 帧 / 2 秒强制全检一次）。
//!
//! 纯 Rust、无 unsafe、无新增依赖；下采样 64×36 灰度，盒式平均。
//! 统计（跳过/总帧比例）由 [`MotionStats`] 暴露，供 `/api/system` 或相机状态读取。

/// 下采样目标尺寸（宽 × 高）。
const DW: usize = 64;
const DH: usize = 36;
/// 默认心跳：每 10 帧强制全检一次（analysis_fps≈5 时约 2 秒）。
pub const DEFAULT_HEARTBEAT_FRAMES: u64 = 10;
/// 默认帧差阈值（0..=255 的平均绝对差）。
pub const DEFAULT_MOTION_THRESHOLD: f32 = 2.0;

#[derive(Debug, Clone, PartialEq)]
pub struct MotionConfig {
    pub enabled: bool,
    /// 平均绝对差阈值；越小越敏感（越不容易跳过）。
    pub threshold: f32,
    /// 距上次全检的最大帧数；到达即强制全检。
    pub heartbeat_frames: u64,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            enabled: false, // 默认关闭——先保证行为与旧版一致，用户可在网页开启
            threshold: DEFAULT_MOTION_THRESHOLD,
            heartbeat_frames: DEFAULT_HEARTBEAT_FRAMES,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MotionStats {
    pub total_frames: u64,
    pub skipped_frames: u64,
}

impl MotionStats {
    /// 跳过帧比例（0.0..=1.0）；无帧时 0.0。
    pub fn skip_ratio(&self) -> f64 {
        if self.total_frames == 0 {
            0.0
        } else {
            self.skipped_frames as f64 / self.total_frames as f64
        }
    }
}

/// 帧差运动门。非线程共享（每路相机一个），纯确定性便于单测。
pub struct MotionGate {
    cfg: MotionConfig,
    prev: Option<[u8; DW * DH]>,
    since_full: u64,
    stats: MotionStats,
}

impl MotionGate {
    pub fn new(cfg: MotionConfig) -> Self {
        Self {
            cfg,
            prev: None,
            since_full: 0,
            stats: MotionStats::default(),
        }
    }

    pub fn config(&self) -> &MotionConfig {
        &self.cfg
    }

    /// 运行时更新配置（网页开关/调阈值即时生效）。变更阈值不重置历史，但
    /// 关闭时清空上一帧，避免重新开启后拿旧帧比对误跳。
    pub fn set_config(&mut self, cfg: MotionConfig) {
        if cfg.enabled != self.cfg.enabled {
            self.prev = None;
            self.since_full = 0;
        }
        self.cfg = cfg;
    }

    pub fn stats(&self) -> &MotionStats {
        &self.stats
    }

    /// 判定当前帧是否可跳过推理。调用方须在 `has_active_tracks=true` 时
    /// 直接全检（本方法亦内置该守卫，双保险）。
    pub fn should_skip(&mut self, bgr: &[u8], w: u32, h: u32, has_active_tracks: bool) -> bool {
        self.stats.total_frames += 1;

        if !self.cfg.enabled || has_active_tracks {
            // 活跃跟踪或停用时清空旧参考帧，避免人离开后拿数分钟前的旧帧误判静止
            self.prev = None;
            self.mark_full();
            return false;
        }
        let gray = downsample_gray(bgr, w, h);
        let skip = match self.prev {
            Some(prev) => {
                let diff = mean_abs_diff(&prev, &gray);
                diff < self.cfg.threshold && self.since_full < self.cfg.heartbeat_frames
            }
            None => false, // 首帧永远全检
        };
        self.prev = Some(gray);

        if skip {
            self.since_full += 1;
            self.stats.skipped_frames += 1;
            true
        } else {
            self.mark_full();
            false
        }
    }

    fn mark_full(&mut self) {
        self.since_full = 0;
    }
}

/// BGR 原始帧（w*h*3）→ 64×36 灰度盒式下采样。
/// 每个目标像素取其覆盖源区域的灰度平均值；越界/尺寸异常返回全 0。
pub fn downsample_gray(bgr: &[u8], w: u32, h: u32) -> [u8; DW * DH] {
    let mut out = [0u8; DW * DH];
    if w == 0 || h == 0 || bgr.len() < (w as usize) * (h as usize) * 3 {
        return out;
    }
    let (w, h) = (w as usize, h as usize);
    for dy in 0..DH {
        let y0 = dy * h / DH;
        let y1 = ((dy + 1) * h / DH).max(y0 + 1);
        for dx in 0..DW {
            let x0 = dx * w / DW;
            let x1 = ((dx + 1) * w / DW).max(x0 + 1);
            let mut sum: u32 = 0;
            let mut n: u32 = 0;
            for y in y0..y1.min(h) {
                let row = y * w * 3;
                for x in x0..x1.min(w) {
                    let i = row + x * 3;
                    // ITU-R BT.601 灰度（BGR 通道顺序：0.114*B + 0.587*G + 0.299*R）
                    let g =
                        (29u32 * bgr[i] as u32 + 150 * bgr[i + 1] as u32 + 77 * bgr[i + 2] as u32)
                            / 256;
                    sum += g;
                    n += 1;
                }
            }
            out[dy * DW + dx] = sum.checked_div(n).unwrap_or(0) as u8;
        }
    }
    out
}

/// 两帧下采样灰度的平均绝对差（0..=255）。
pub fn mean_abs_diff(a: &[u8; DW * DH], b: &[u8; DW * DH]) -> f32 {
    let mut sum: u32 = 0;
    for i in 0..DW * DH {
        sum += a[i].abs_diff(b[i]) as u32;
    }
    sum as f32 / (DW * DH) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, b: u8, g: u8, r: u8) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 3) as usize];
        for i in 0..(w * h) as usize {
            v[i * 3] = b;
            v[i * 3 + 1] = g;
            v[i * 3 + 2] = r;
        }
        v
    }

    fn cfg() -> MotionConfig {
        MotionConfig {
            enabled: true,
            threshold: 2.0,
            heartbeat_frames: 3,
        }
    }

    #[test]
    fn downsample_and_diff_are_deterministic() {
        let a = downsample_gray(&solid(128, 72, 10, 20, 30), 128, 72);
        let b = downsample_gray(&solid(128, 72, 10, 20, 30), 128, 72);
        assert_eq!(mean_abs_diff(&a, &b), 0.0);
        let c = downsample_gray(&solid(128, 72, 200, 200, 200), 128, 72);
        assert!(mean_abs_diff(&a, &c) > 50.0);
    }

    #[test]
    fn static_frame_skips_after_first() {
        let mut gate = MotionGate::new(cfg());
        let f = solid(128, 72, 40, 40, 40);
        assert!(!gate.should_skip(&f, 128, 72, false)); // 首帧全检
        assert!(gate.should_skip(&f, 128, 72, false)); // 静止跳过
        assert!(gate.should_skip(&f, 128, 72, false));
    }

    #[test]
    fn active_track_never_skips() {
        let mut gate = MotionGate::new(cfg());
        let f = solid(128, 72, 40, 40, 40);
        gate.should_skip(&f, 128, 72, false);
        for _ in 0..20 {
            assert!(!gate.should_skip(&f, 128, 72, true)); // 有 track 必全检
        }
        assert_eq!(gate.stats().skipped_frames, 0);
    }

    #[test]
    fn heartbeat_forces_full_check() {
        let mut gate = MotionGate::new(cfg());
        let f = solid(128, 72, 40, 40, 40);
        gate.should_skip(&f, 128, 72, false); // 首帧
                                              // heartbeat=3：连续跳过 3 帧后第 4 帧强制全检
        assert!(gate.should_skip(&f, 128, 72, false));
        assert!(gate.should_skip(&f, 128, 72, false));
        assert!(gate.should_skip(&f, 128, 72, false));
        assert!(!gate.should_skip(&f, 128, 72, false));
        assert_eq!(gate.stats().skipped_frames, 3);
    }

    #[test]
    fn changed_frame_never_skips() {
        let mut gate = MotionGate::new(cfg());
        gate.should_skip(&solid(128, 72, 0, 0, 0), 128, 72, false);
        let bright = solid(128, 72, 255, 255, 255);
        assert!(!gate.should_skip(&bright, 128, 72, false));
    }

    #[test]
    fn disabled_always_full_check() {
        let mut c = cfg();
        c.enabled = false;
        let mut gate = MotionGate::new(c);
        let f = solid(64, 36, 1, 2, 3);
        for _ in 0..10 {
            assert!(!gate.should_skip(&f, 64, 36, false));
        }
        assert_eq!(gate.stats().skip_ratio(), 0.0);
    }

    #[test]
    fn runtime_toggle_resets_previous_frame() {
        let mut gate = MotionGate::new(cfg());
        let f = solid(64, 36, 9, 9, 9);
        gate.should_skip(&f, 64, 36, false);
        let mut off = cfg();
        off.enabled = false;
        gate.set_config(off);
        gate.set_config(cfg());
        // 重开后第一帧应全检（prev 被清空）
        assert!(!gate.should_skip(&f, 64, 36, false));
    }
}
