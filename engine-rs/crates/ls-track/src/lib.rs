//! ByteTrack 主循环：supervision 0.25.1 byte_tracker/core.py 逐函数移植（T-324）。
//!
//! 对齐要点（与 Python 实现一致才有 L1 轨迹一致率 ≥99.5%）：
//! - `linear_assignment`：scipy linear_sum_assignment 的最优指派——这里实现 Kuhn-Munkres
//!   （O(n³)），代价矩阵先做 `>thresh → thresh+1e-4` 钳制，再按 thresh 截断匹配；
//! - 两轮关联（高/低置信度）、unconfirmed 第三轮、fuse_score、去重，全部同序。

// Kuhn-Munkres 与匹配循环用索引实现（矩阵形状小、对齐审计优先）
#![allow(clippy::needless_range_loop)]

pub mod kalman;

pub const NO_ID: i64 = -1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackState {
    New = 0,
    Tracked = 1,
    Lost = 2,
    Removed = 3,
}

#[derive(Clone, Copy, Debug)]
pub struct Detection {
    /// tlbr
    pub box_: [f32; 4],
    pub score: f32,
}

#[derive(Clone)]
pub struct STrack {
    pub state: TrackState,
    pub is_activated: bool,
    pub start_frame: u64,
    pub frame_id: u64,
    /// tlwh（未激活时的原始观测）
    raw_tlwh: [f32; 4],
    mean: Option<[f64; 8]>,
    covariance: Option<kalman::Mat8>,
    pub score: f32,
    tracklet_len: u32,
    minimum_consecutive_frames: u32,
    pub internal_track_id: i64,
    pub external_track_id: i64,
    /// 移植语义：Python 里 activate 用专属 KF、multi_predict 用 shared KF——
    /// 两者数学等价（同一 F/Q/R 常量），Rust 统一无状态函数即可。
    _kalman_priv: (),
}

impl STrack {
    fn new(tlbr: [f32; 4], score: f32, min_consecutive: u32) -> Self {
        STrack {
            state: TrackState::New,
            is_activated: false,
            start_frame: 0,
            frame_id: 0,
            raw_tlwh: tlbr_to_tlwh(tlbr),
            mean: None,
            covariance: None,
            score,
            tracklet_len: 0,
            minimum_consecutive_frames: min_consecutive,
            internal_track_id: NO_ID,
            external_track_id: NO_ID,
            _kalman_priv: (),
        }
    }

    fn tlwh(&self) -> [f32; 4] {
        match self.mean {
            None => self.raw_tlwh,
            Some(m) => {
                let mut ret = [m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32];
                ret[2] *= ret[3];
                ret[0] -= ret[2] / 2.0;
                ret[1] -= ret[3] / 2.0;
                ret
            }
        }
    }

    fn tlbr(&self) -> [f32; 4] {
        let t = self.tlwh();
        [t[0], t[1], t[0] + t[2], t[1] + t[3]]
    }

    fn activate(&mut self, frame_id: u64, internal_counter: &mut i64, external_counter: &mut i64) {
        self.internal_track_id = *internal_counter;
        *internal_counter += 1;
        let xyah = tlwh_to_xyah(self.raw_tlwh);
        self.mean = Some(initiate(xyah));
        self.covariance = Some(init_covariance(xyah));
        self.tracklet_len = 0;
        self.state = TrackState::Tracked;
        self.is_activated = frame_id == 1;
        if self.minimum_consecutive_frames == 1 {
            self.external_track_id = *external_counter;
            *external_counter += 1;
        }
        self.frame_id = frame_id;
        self.start_frame = frame_id;
    }

    fn re_activate(&mut self, det: &Detection, frame_id: u64) {
        let m = self.mean.take().unwrap();
        let c = self.covariance.take().unwrap();
        let tlwh64: [f64; 4] = tlbr_to_tlwh(det.box_).map(|v| v as f64);
        let (nm, nc) = kalman::update(m, &c, tlwh_to_xyah_f64(tlwh64));
        self.mean = Some(nm);
        self.covariance = Some(nc);
        self.tracklet_len = 0;
        self.state = TrackState::Tracked;
        self.frame_id = frame_id;
        self.score = det.score;
    }

    fn update(&mut self, det: &Detection, frame_id: u64, external_counter: &mut i64) {
        self.frame_id = frame_id;
        self.tracklet_len += 1;
        let m = self.mean.take().unwrap();
        let c = self.covariance.take().unwrap();
        let tlwh64: [f64; 4] = tlbr_to_tlwh(det.box_).map(|v| v as f64);
        let (nm, nc) = kalman::update(m, &c, tlwh_to_xyah_f64(tlwh64));
        self.mean = Some(nm);
        self.covariance = Some(nc);
        self.state = TrackState::Tracked;
        if self.tracklet_len == self.minimum_consecutive_frames {
            self.is_activated = true;
            if self.external_track_id == NO_ID {
                self.external_track_id = *external_counter;
                *external_counter += 1;
            }
        }
        self.score = det.score;
    }

    /// 预测一步（Python multi_predict 的逐对象等价；Tracked 之外 vh 先清零）
    fn multi_predict(t: &mut STrack) {
        let m = t.mean.take().unwrap();
        let c = t.covariance.take().unwrap();
        let mut m2 = m;
        if t.state != TrackState::Tracked {
            m2[7] = 0.0;
        }
        let (nm, nc) = kalman::predict(m2, &c);
        t.mean = Some(nm);
        t.covariance = Some(nc);
    }
}

/// KF.initiate 的均值部分（位置=观测，速度=0）
fn initiate(xyah: [f32; 4]) -> [f64; 8] {
    [
        xyah[0] as f64,
        xyah[1] as f64,
        xyah[2] as f64,
        xyah[3] as f64,
        0.0,
        0.0,
        0.0,
        0.0,
    ]
}

pub(crate) fn kalman_std_position() -> f64 {
    1.0 / 20.0
}

pub(crate) fn kalman_std_velocity() -> f64 {
    1.0 / 160.0
}

/// KF.initiate 的协方差部分
fn init_covariance(xyah: [f32; 4]) -> kalman::Mat8 {
    let h = xyah[3] as f64;
    let std = [
        2.0 * kalman_std_position() * h,
        2.0 * kalman_std_position() * h,
        1e-2,
        2.0 * kalman_std_position() * h,
        10.0 * kalman_std_velocity() * h,
        10.0 * kalman_std_velocity() * h,
        1e-5,
        10.0 * kalman_std_velocity() * h,
    ];
    kalman::Mat8::diag(std.map(|v| v * v))
}

pub(crate) fn tlwh_to_xyah_f64(tlwh: [f64; 4]) -> [f64; 4] {
    let w = tlwh[2].max(1e-6);
    let x = tlwh[0] + w / 2.0;
    let y = tlwh[1] + tlwh[3] / 2.0;
    [x, y, w / tlwh[3].max(1e-6), tlwh[3]]
}

pub fn tlbr_to_tlwh(b: [f32; 4]) -> [f32; 4] {
    [b[0], b[1], b[2] - b[0], b[3] - b[1]]
}

pub fn tlwh_to_xyah(t: [f32; 4]) -> [f32; 4] {
    [t[0] + t[2] / 2.0, t[1] + t[3] / 2.0, t[2] / t[3], t[3]]
}

fn box_iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let area_a = (a[2] - a[0]) * (a[3] - a[1]);
    let area_b = (b[2] - b[0]) * (b[3] - b[1]);
    let tl_x = a[0].max(b[0]);
    let tl_y = a[1].max(b[1]);
    let br_x = (a[2]).min(b[2]);
    let br_y = (a[3]).min(b[3]);
    let inter = (br_x - tl_x).max(0.0) * (br_y - tl_y).max(0.0);
    let denom = area_a + area_b - inter;
    if denom <= 0.0 {
        0.0
    } else {
        inter / denom
    }
}

fn iou_cost_matrix(a: &[[f32; 4]], b: &[[f32; 4]]) -> Vec<Vec<f32>> {
    a.iter()
        .map(|x| b.iter().map(|y| 1.0 - box_iou(*x, *y)).collect())
        .collect()
}

fn fuse_score(mut cost: Vec<Vec<f32>>, dets: &[f32]) -> Vec<Vec<f32>> {
    if cost.is_empty() || dets.is_empty() {
        return cost;
    }
    for row in cost.iter_mut() {
        for (j, cell) in row.iter_mut().enumerate() {
            let iou_sim = 1.0 - *cell;
            *cell = 1.0 - iou_sim * dets[j];
        }
    }
    cost
}

/// Kuhn-Munkres 最优指派（方阵化 padding，O(n³)），等价 scipy linear_sum_assignment。
fn linear_sum_assignment(cost: &[Vec<f32>]) -> Vec<(usize, usize)> {
    let n = cost.len().max(cost.first().map(|r| r.len()).unwrap_or(0));
    if n == 0 {
        return vec![];
    }
    const INF: f32 = f32::INFINITY;
    // 非方阵填充：用大代价（而非 0）占位——否则虚拟行会以 0 代价抢走真实列，
    // 与 scipy linear_sum_assignment 的矩形语义不一致（真实检测回归发现）。
    const PAD: f32 = 1e6;
    let mut a = vec![vec![PAD; n]; n];
    for i in 0..cost.len() {
        for j in 0..cost[i].len() {
            a[i][j] = cost[i][j];
        }
    }
    let mut u = vec![0.0f32; n + 1];
    let mut v = vec![0.0f32; n + 1];
    let mut p = vec![0usize; n + 1];
    let mut way = vec![0usize; n + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![INF; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = INF;
            let mut j1 = 0usize;
            for j in 1..=n {
                if !used[j] {
                    let cur = a[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut out = vec![];
    for j in 1..=n {
        if p[j] != 0 {
            out.push((p[j] - 1, j - 1));
        }
    }
    out
}

/// matching.linear_assignment：钳制 + 最优指派 + 阈值截断。
fn linear_assignment_thresh(
    cost: Vec<Vec<f32>>,
    cols: usize,
    thresh: f32,
) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    let rows = cost.len();
    // cols 由调用方显式传入：空池时 cost 形状 (0,0)，first() 推不出列数
    //（对应 scipy 对 (0,N) 矩阵 unmatched_b = 全部检测的语义）
    if rows == 0 || cols == 0 {
        return (vec![], (0..rows).collect(), (0..cols).collect());
    }
    let mut clamped = cost;
    for row in clamped.iter_mut() {
        for c in row.iter_mut() {
            if *c > thresh {
                *c = thresh + 1e-4;
            }
        }
    }
    let indices = linear_sum_assignment(&clamped);
    let mut matches = vec![];
    let mut ua: Vec<usize> = (0..rows).collect();
    let mut ub: Vec<usize> = (0..cols).collect();
    for (i, j) in indices {
        // KM 输出含方阵填充区（i>=rows 或 j>=cols），只接受真实行列
        if i < rows && j < cols && clamped[i][j] <= thresh {
            matches.push((i, j));
        }
    }
    ua.retain(|i| !matches.iter().any(|m| m.0 == *i));
    ub.retain(|j| !matches.iter().any(|m| m.1 == *j));
    (matches, ua, ub)
}

/// ByteTrack 参数（与 supervision 默认/LS 配置对应）。
#[derive(Clone, Copy, Debug)]
pub struct ByteTrackConfig {
    pub track_activation_threshold: f32,
    pub lost_track_buffer: u32,
    pub minimum_matching_threshold: f32,
    pub frame_rate: u32,
    pub minimum_consecutive_frames: u32,
}

impl Default for ByteTrackConfig {
    fn default() -> Self {
        ByteTrackConfig {
            track_activation_threshold: 0.25,
            lost_track_buffer: 30,
            minimum_matching_threshold: 0.8,
            frame_rate: 30,
            minimum_consecutive_frames: 1,
        }
    }
}

/// ByteTrack（与 sv.ByteTrack 语义一致；输出为"本帧激活轨迹"的 (tlbr, external_id)）。
/// (external_id, mean_x) 快照元组（L1 对齐诊断用）
pub type TrackSnapEntry = (i64, f64);
/// (tracked, lost, unconfirmed) 快照
pub type TrackStateSnap = (
    Vec<TrackSnapEntry>,
    Vec<TrackSnapEntry>,
    Vec<TrackSnapEntry>,
);

pub struct ByteTrack {
    cfg: ByteTrackConfig,
    frame_id: u64,
    det_thresh: f32,
    max_time_lost: u64,
    tracked: Vec<STrack>,
    lost: Vec<STrack>,
    removed: Vec<STrack>,
    internal_counter: i64,
    external_counter: i64,
}

impl ByteTrack {
    pub fn new(cfg: ByteTrackConfig) -> Self {
        ByteTrack {
            det_thresh: cfg.track_activation_threshold + 0.1,
            max_time_lost: ((cfg.frame_rate as f32 / 30.0) * cfg.lost_track_buffer as f32) as u64,
            frame_id: 0,
            tracked: vec![],
            lost: vec![],
            removed: vec![],
            internal_counter: 0,
            external_counter: 1,
            cfg,
        }
    }

    /// L1 对齐诊断：tracked/lost/unconfirmed 的 (external_id, mean_x) 快照。
    pub fn state_snapshot(&self) -> TrackStateSnap {
        let snap = |v: &[STrack]| -> Vec<(i64, f64)> {
            v.iter()
                .map(|t| (t.external_track_id, t.mean.map(|m| m[0]).unwrap_or(0.0)))
                .collect()
        };
        let unconf: Vec<STrack> = self
            .tracked
            .iter()
            .filter(|t| !t.is_activated)
            .cloned()
            .collect();
        (snap(&self.tracked), snap(&self.lost), snap(&unconf))
    }

    /// 输入一批检测（tlbr+score），返回激活轨迹 [(tlbr, external_id)]。
    pub fn update(&mut self, detections: &[Detection]) -> Vec<([f32; 4], i64)> {
        self.frame_id += 1;
        let frame_id = self.frame_id;
        let mut activated: Vec<usize> = vec![];
        let mut refind: Vec<usize> = vec![];

        // L1：输入检测序 → track external_id（supervision 语义：输出=输入检测框原样，
        // IoU>=0.5 回填 id；未关联检测被丢弃）
        let mut det_assign: Vec<Option<i64>> = vec![None; detections.len()];
        let mut dets_high: Vec<(usize, Detection)> = vec![];
        let mut dets_second: Vec<(usize, Detection)> = vec![];
        for (di, d) in detections.iter().enumerate() {
            if d.score > self.cfg.track_activation_threshold {
                dets_high.push((di, *d));
            } else if d.score > 0.1 && d.score < self.cfg.track_activation_threshold {
                dets_second.push((di, *d));
            }
        }

        let mut unconfirmed_idx: Vec<usize> = vec![];
        let mut tracked_idx: Vec<usize> = vec![];
        for (i, t) in self.tracked.iter().enumerate() {
            if !t.is_activated {
                unconfirmed_idx.push(i);
            } else {
                tracked_idx.push(i);
            }
        }

        // Step 2: 第一轮（高分 vs tracked+lost）
        // 把 tracked/lost 的 STrack 移入池（克隆语义不适用——Python 是引用共享，
        // 这里用索引操作 self.tracked/self.lost 保持同一对象）
        let mut pool: Vec<(usize, bool)> = tracked_idx.iter().map(|&i| (i, true)).collect();
        pool.extend(self.lost.iter().enumerate().map(|(i, _)| (i, false)));

        // 预测：对 tracked+lost 全部（multi_predict 语义）
        for (i, is_tracked) in pool.iter() {
            let t = if *is_tracked {
                &mut self.tracked[*i]
            } else {
                &mut self.lost[*i]
            };
            STrack::multi_predict(t);
        }
        let pool_tlbrs: Vec<[f32; 4]> = pool
            .iter()
            .map(|(i, is_t)| {
                if *is_t {
                    self.tracked[*i].tlbr()
                } else {
                    self.lost[*i].tlbr()
                }
            })
            .collect();
        let det_tlbrs: Vec<[f32; 4]> = dets_high.iter().map(|(_, d)| d.box_).collect();
        let det_scores: Vec<f32> = dets_high.iter().map(|(_, d)| d.score).collect();
        let cost = fuse_score(iou_cost_matrix(&pool_tlbrs, &det_tlbrs), &det_scores);
        let (matches, u_track, u_detection) =
            linear_assignment_thresh(cost, dets_high.len(), self.cfg.minimum_matching_threshold);
        if std::env::var("LS_DEBUG_MATCH").is_ok() {
            eprintln!(
                "[match] frame={frame_id} pool={:?} dets={:?} matches={matches:?} u_track={u_track:?} u_det={u_detection:?}",
                pool.iter().map(|(i, tr)| {
                    let t = if *tr { &self.tracked[*i] } else { &self.lost[*i] };
                    (t.external_track_id, t.state as i32)
                })
                .collect::<Vec<_>>(),
                dets_high.iter().map(|(i, d)| (*i, d.score)).collect::<Vec<_>>(),
            );
        }

        for (itracked, idet) in matches {
            let (i, is_t) = pool[itracked];
            let t = if is_t {
                &mut self.tracked[i]
            } else {
                &mut self.lost[i]
            };
            if t.state == TrackState::Tracked {
                t.update(&dets_high[idet].1, frame_id, &mut self.external_counter);
                det_assign[dets_high[idet].0] = Some(t.external_track_id);
                if is_t {
                    activated.push(i);
                }
            } else {
                t.re_activate(&dets_high[idet].1, frame_id);
                det_assign[dets_high[idet].0] = Some(t.external_track_id);
                if is_t {
                    refind.push(i);
                }
            }
        }

        // Step 3: 第二轮（低分 vs 未匹配的 Tracked）
        let r_tracked: Vec<(usize, bool)> = u_track
            .iter()
            .map(|&k| pool[k])
            .filter(|(i, is_t)| {
                if *is_t {
                    self.tracked[*i].state == TrackState::Tracked
                } else {
                    self.lost[*i].state == TrackState::Tracked
                }
            })
            .collect();
        let r_tlbrs: Vec<[f32; 4]> = r_tracked
            .iter()
            .map(|(i, is_t)| {
                if *is_t {
                    self.tracked[*i].tlbr()
                } else {
                    self.lost[*i].tlbr()
                }
            })
            .collect();
        let second_tlbrs: Vec<[f32; 4]> = dets_second.iter().map(|(_, d)| d.box_).collect();
        let cost2 = iou_cost_matrix(&r_tlbrs, &second_tlbrs);
        let (matches2, u_track2, _u_det2) = linear_assignment_thresh(cost2, dets_second.len(), 0.5);
        for (itracked, idet) in matches2 {
            let (i, is_t) = r_tracked[itracked];
            let t = if is_t {
                &mut self.tracked[i]
            } else {
                &mut self.lost[i]
            };
            if t.state == TrackState::Tracked {
                t.update(&dets_second[idet].1, frame_id, &mut self.external_counter);
                det_assign[dets_second[idet].0] = Some(t.external_track_id);
                if is_t {
                    activated.push(i);
                }
            } else {
                t.re_activate(&dets_second[idet].1, frame_id);
                det_assign[dets_second[idet].0] = Some(t.external_track_id);
                if is_t {
                    refind.push(i);
                }
            }
        }
        for &k in u_track2.iter() {
            let (i, is_t) = r_tracked[k];
            let t = if is_t {
                &mut self.tracked[i]
            } else {
                &mut self.lost[i]
            };
            if t.state != TrackState::Lost {
                t.state = TrackState::Lost;
            }
        }

        // 第三轮：unconfirmed vs 剩余高分
        let remaining: Vec<(usize, Detection)> =
            u_detection.iter().map(|&i| dets_high[i]).collect();
        let unconf_tlbrs: Vec<[f32; 4]> = unconfirmed_idx
            .iter()
            .map(|&i| self.tracked[i].tlbr())
            .collect();
        let rem_tlbrs: Vec<[f32; 4]> = remaining.iter().map(|(_, d)| d.box_).collect();
        let rem_scores: Vec<f32> = remaining.iter().map(|(_, d)| d.score).collect();
        let cost3 = fuse_score(iou_cost_matrix(&unconf_tlbrs, &rem_tlbrs), &rem_scores);
        if std::env::var("LS_DEBUG_MATCH").is_ok() {
            eprintln!(
                "[round3] frame={frame_id} unconf={:?} rem={:?} cost3={:?}",
                unconfirmed_idx
                    .iter()
                    .map(|&i| (
                        self.tracked[i].external_track_id,
                        self.tracked[i].is_activated
                    ))
                    .collect::<Vec<_>>(),
                remaining
                    .iter()
                    .map(|(i, d)| (*i, d.score))
                    .collect::<Vec<_>>(),
                cost3
                    .iter()
                    .map(|r| r
                        .iter()
                        .map(|v| (v * 100.0).round() / 100.0)
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            );
        }
        let (matches3, u_unconf, u_det3) = linear_assignment_thresh(cost3, remaining.len(), 0.7);
        for (itracked, idet) in matches3 {
            let i = unconfirmed_idx[itracked];
            self.tracked[i].update(&remaining[idet].1, frame_id, &mut self.external_counter);
            det_assign[remaining[idet].0] = Some(self.tracked[i].external_track_id);
            activated.push(i);
        }
        for &k in u_unconf.iter() {
            self.tracked[unconfirmed_idx[k]].state = TrackState::Removed;
        }

        // Step 4: 新建
        for &k in u_det3.iter() {
            if remaining[k].1.score < self.det_thresh {
                continue;
            }
            let mut track = STrack::new(
                remaining[k].1.box_,
                remaining[k].1.score,
                self.cfg.minimum_consecutive_frames,
            );
            track.activate(
                frame_id,
                &mut self.internal_counter,
                &mut self.external_counter,
            );
            // supervision：activate 的新 track 首帧 is_activated=False（frame_id!=1），
            // 不进输出（下一帧 update 达标后才输出）——检测与 Python 语义一致
            if std::env::var("LS_DEBUG_MATCH").is_ok() {
                eprintln!(
                    "[step4] frame={frame_id} new_track ext={} activated={} box={:?}",
                    track.external_track_id, track.is_activated, remaining[k].1.box_
                );
            }
            if track.is_activated {
                det_assign[remaining[k].0] = Some(track.external_track_id);
            }
            self.tracked.push(track);
        }

        // Step 5: 超时移除
        for t in self.lost.iter_mut() {
            if frame_id.saturating_sub(t.frame_id) > self.max_time_lost {
                t.state = TrackState::Removed;
            }
        }

        // 状态收敛（对应 Python 末段 tracked/lost/removed 重组 + joint/sub 语义）：
        // tracked 里转 Lost/Removed 的移出；lost 里 re_activate 成 Tracked 的移回
        let mut still_tracked: Vec<STrack> = vec![];
        let mut to_lost: Vec<STrack> = vec![];
        let mut to_removed: Vec<STrack> = vec![];
        for t in self.tracked.drain(..) {
            match t.state {
                TrackState::Tracked => still_tracked.push(t),
                TrackState::Lost => to_lost.push(t),
                TrackState::Removed => to_removed.push(t),
                TrackState::New => still_tracked.push(t),
            }
        }
        let mut still_lost: Vec<STrack> = vec![];
        for t in self.lost.drain(..) {
            match t.state {
                TrackState::Tracked => still_tracked.push(t), // refind
                TrackState::Lost => still_lost.push(t),
                TrackState::Removed => to_removed.push(t),
                TrackState::New => still_lost.push(t),
            }
        }
        self.lost = still_lost;
        self.lost.extend(to_lost);
        self.removed = to_removed;
        self.tracked = still_tracked; // refind 的轨迹已在 drain lost 时并回
        remove_duplicate_tracks(&mut self.tracked, &mut self.lost);

        // supervision update_with_detections：返回**输入检测框原样** × tracker_id
        //（IoU>=0.5 匹配回填；未关联检测丢弃——Kalman 框仅内部使用）
        let out: Vec<([f32; 4], i64)> = detections
            .iter()
            .zip(det_assign.iter())
            .filter_map(|(d, id)| id.as_ref().map(|id| (d.box_, *id)))
            .collect();
        out
    }
}

/// Python remove_duplicate_tracks：tracked×lost 中 IoU<0.15 的重叠对，
/// 保留存活时间长者（frame_id - start_frame），短者按所在列表移除。
fn remove_duplicate_tracks(tracks_a: &mut Vec<STrack>, tracks_b: &mut Vec<STrack>) {
    let mut dup_a: Vec<usize> = vec![];
    let mut dup_b: Vec<usize> = vec![];
    for (ia, ta) in tracks_a.iter().enumerate() {
        for (ib, tb) in tracks_b.iter().enumerate() {
            // Python：iou_distance < 0.15 才算重复（即 IoU > 0.85）——
            // 原实现写反成 IoU>=0.15 就去重，导致拥挤场景大量轨迹被误删（L1 黄金语料实测）
            if box_iou(ta.tlbr(), tb.tlbr()) < 0.85 {
                continue;
            }
            let time_a = ta.frame_id.saturating_sub(ta.start_frame);
            let time_b = tb.frame_id.saturating_sub(tb.start_frame);
            if time_a > time_b {
                dup_b.push(ib);
            } else {
                dup_a.push(ia);
            }
        }
    }
    let mut idx = 0usize;
    tracks_a.retain(|_| {
        let keep = !dup_a.contains(&idx);
        idx += 1;
        keep
    });
    let mut idx = 0usize;
    tracks_b.retain(|_| {
        let keep = !dup_b.contains(&idx);
        idx += 1;
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_box_stable_id() {
        let mut bt = ByteTrack::new(ByteTrackConfig::default());
        let det = Detection {
            box_: [100.0, 100.0, 300.0, 500.0],
            score: 0.9,
        };
        let out = bt.update(&[det]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, 1);
        for _ in 0..10 {
            let out = bt.update(&[Detection {
                box_: [102.0, 100.0, 302.0, 500.0],
                score: 0.9,
            }]);
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].1, 1, "同一物体持续出现 → external_id 不变");
        }
    }

    #[test]
    fn occlusion_gap_keeps_id() {
        let mut bt = ByteTrack::new(ByteTrackConfig {
            lost_track_buffer: 30,
            frame_rate: 30,
            ..Default::default()
        });
        let det = Detection {
            box_: [100.0, 100.0, 300.0, 500.0],
            score: 0.9,
        };
        bt.update(&[det]);
        for _ in 0..10 {
            bt.update(&[]); // 遮挡 10 帧
        }
        let out = bt.update(&[Detection {
            box_: [110.0, 100.0, 310.0, 500.0],
            score: 0.9,
        }]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].1, 1, "lost_track_buffer 内重现 → 同 id");
    }

    #[test]
    fn rectangular_assignment_ignores_padding() {
        // 1 行 × 3 列：真实匹配应为代价最小的 (0,1)；填充区不得越界/抢列
        let cost = vec![vec![0.9f32, 0.1, 0.8]];
        let (m, ua, ub) = linear_assignment_thresh(cost, 3, 0.3);
        assert_eq!(m, vec![(0, 1)]);
        assert!(ua.is_empty());
        assert_eq!(ub, vec![0, 2]);
        // 2 行 × 3 列：两行各取最近列，不 panic
        let cost = vec![vec![0.1, 0.9, 0.9], vec![0.9, 0.1, 0.9]];
        let (m, _, _) = linear_assignment_thresh(cost, 3, 0.3);
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|(i, _)| *i < 2));
    }
}
