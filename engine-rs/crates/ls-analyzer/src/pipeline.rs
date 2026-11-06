//! 单路分析管线（T-331 数据面）：检测 → ByteTrack → 区域迟滞 → 规则/冷却 → 事件候选。
//!
//! 与 Python `pipeline.py::_analyze` + `rules.py::RuleEngine` 对齐：
//! - 区域锚点 = 检测框底边中点（`(x1+x2)/2, y2` 取整）；`point_in_polygon` 边界算内；
//! - 每帧对每个 zone 更新 `ZoneState`（enter/exit 帧数可配）；无检出帧对全部在内 track
//!   补离场观测（避免单帧遮挡导致 exit 永不触发）；
//! - `occupancy_over` 按区去重人数；rule.zone_id=0 用"任一区去重人数"（画面口径）；
//! - 冷却键 (rule_id, track_id)，以分析帧序号为单调时钟（与 Python 确定性模式一致）；
//! - 时段（支持跨午夜）/ weekday（"1-5" 或 "1,2,3"）/ 日历例外（holiday/workday）。
//!
//! 检测来源通过 [`Detector`] 抽象：T-325 ort 就绪前使用 [`NullDetector`]（零事件），
//! 推理层替换后本模块无需改动。

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone};

use ls_core::state::ZoneState;
use ls_core::{point_in_polygon, ZoneChange};
use ls_track::{ByteTrack, ByteTrackConfig, Detection as TrackDetection};

#[derive(Clone, Debug)]
#[allow(dead_code)] // name 用于诊断/后续快照展示；数据面无读取
pub struct CameraSpec {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub type_: String,
    pub analysis_fps: f32,
    pub file_loop: bool,
}

#[derive(Clone, Debug)]
pub struct ZoneSpec {
    pub id: i64,
    pub camera_id: i64,
    pub name: String,
    pub points: Vec<(i64, i64)>,
}

#[derive(Clone, Debug)]
pub struct RuleSpec {
    pub id: i64,
    pub camera_id: i64,
    /// 0 = 整画面（occupancy 口径）；enter/exit 规则 0 视为任意区域
    pub zone_id: i64,
    pub behavior: String,
    pub level: i64,
    pub cooldown_s: i64,
    pub time_start: String,
    pub time_end: String,
    /// "1-5" 或 "1,2,3"；空=全部
    pub weekdays: String,
    pub threshold: i64,
    pub enabled: bool,
    /// proto v1 尚无该字段（保留供后续契约扩展）
    pub holiday_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Wearing/NoPpe 由 T-325 检层构造；管线分支已就绪
pub enum PpeStatus {
    Unknown,
    Wearing,
    NoPpe,
}

#[derive(Clone, Copy, Debug)]
pub struct PersonDetection {
    /// tlbr（像素）
    pub box_: [f32; 4],
    pub score: f32,
    pub ppe: PpeStatus,
}

/// 检测器抽象（T-325 接入 ort 后替换实现）。
pub trait Detector: Send {
    /// 检测一帧（BGR raw），返回人员框 + 实验服状态。
    fn detect(&mut self, frame: &ls_ingest::Frame) -> Vec<PersonDetection>;
}

/// 无推理后端：保留管线/事件通道，不产事件（T-325 前）。
pub struct NullDetector;

impl Detector for NullDetector {
    fn detect(&mut self, _frame: &ls_ingest::Frame) -> Vec<PersonDetection> {
        Vec::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub camera_id: i64,
    pub zone_id: Option<i64>,
    pub behavior: String,
    pub level: i64,
    pub track_id: i64,
    pub detail: String,
    pub ts_iso: String,
    pub evidence_key: String,
}

pub struct CameraPipeline {
    camera: CameraSpec,
    zones: Vec<ZoneSpec>,
    rules: Vec<RuleSpec>,
    calendar: HashMap<String, String>,
    tracker: ByteTrack,
    zone_states: HashMap<i64, ZoneState>,
    /// (rule_id, track_id) -> 最近触发帧序号
    last_fire: HashMap<(i64, i64), u64>,
    frame_index: u64,
}

impl CameraPipeline {
    pub fn new(
        camera: CameraSpec,
        zones: Vec<ZoneSpec>,
        rules: Vec<RuleSpec>,
        calendar: HashMap<String, String>,
        enter_frames: u32,
        exit_frames: u32,
    ) -> Self {
        let fps = camera.analysis_fps.max(1.0);
        let tracker = ByteTrack::new(ByteTrackConfig {
            frame_rate: fps.round() as u32,
            lost_track_buffer: (fps * 3.0) as u32,
            ..ByteTrackConfig::default()
        });
        let mut zone_states = HashMap::new();
        for z in &zones {
            zone_states.insert(
                z.id,
                ZoneState::new(ls_core::state::Hysteresis {
                    enter_frames,
                    exit_frames,
                }),
            );
        }
        Self {
            camera,
            zones,
            rules,
            calendar,
            tracker,
            zone_states,
            last_fire: HashMap::new(),
            frame_index: 0,
        }
    }

    /// 处理一帧检测结果，返回本帧事件候选（未入库）。
    pub fn process(&mut self, dets: &[PersonDetection], frame_ts: &str) -> Vec<Candidate> {
        self.frame_index += 1;
        let now = parse_ts(frame_ts).unwrap_or_else(Local::now);
        let track_dets: Vec<TrackDetection> = dets
            .iter()
            .map(|d| TrackDetection {
                box_: d.box_,
                score: d.score,
            })
            .collect();
        let active = self.tracker.update(&track_dets);
        let active_ids: HashSet<i64> = active.iter().map(|(_, id)| *id).collect();
        let mut out = Vec::new();

        // 本帧未再出现的在场 track：补离场观测（单帧漏检不立即翻转为 exit——迟滞生效）
        let mut exits: Vec<(i64, i64)> = Vec::new();
        for (zid, st) in self.zone_states.iter_mut() {
            let gone: Vec<i64> = st
                .inside_ids()
                .into_iter()
                .filter(|t| !active_ids.contains(t))
                .collect();
            for tid in gone {
                if st.update(tid, false) == Some(ZoneChange::Exit) {
                    exits.push((*zid, tid));
                }
            }
        }
        if dets.is_empty() {
            for st in self.zone_states.values_mut() {
                st.retain(&[], None);
            }
        }
        for (zid, tid) in exits {
            let detail = format!("离开安全区「{}」", self.zone_name(zid));
            self.try_fire(Some(zid), "exit", tid, &detail, 2, &now, frame_ts, &mut out);
        }

        // 按区去重人数（occupancy 口径）
        let mut occ: HashMap<i64, HashSet<i64>> =
            self.zones.iter().map(|z| (z.id, HashSet::new())).collect();
        let mut changes: Vec<(i64, i64, ZoneChange, PpeStatus)> = Vec::new();
        for (bbox, tid) in &active {
            let anchor = (((bbox[0] + bbox[2]) / 2.0) as i64, (bbox[3]) as i64);
            let inside: HashSet<i64> = self
                .zones
                .iter()
                .filter(|z| point_in_polygon(&z.points, anchor))
                .map(|z| z.id)
                .collect();
            for zid in &inside {
                if let Some(s) = occ.get_mut(zid) {
                    s.insert(*tid);
                }
            }
            let ppe = dets
                .iter()
                .find(|d| d.box_ == *bbox)
                .map(|d| d.ppe)
                .unwrap_or(PpeStatus::Unknown);
            for z in &self.zones {
                if let Some(change) = self
                    .zone_states
                    .get_mut(&z.id)
                    .and_then(|st| st.update(*tid, inside.contains(&z.id)))
                {
                    changes.push((z.id, *tid, change, ppe));
                }
            }
        }
        for (zid, tid, change, ppe) in changes {
            let zname = self.zone_name(zid);
            match change {
                ZoneChange::Exit => {
                    let detail = format!("离开安全区「{zname}」");
                    self.try_fire(Some(zid), "exit", tid, &detail, 2, &now, frame_ts, &mut out);
                }
                ZoneChange::Enter => match ppe {
                    PpeStatus::Wearing => {
                        let detail = format!("进入安全区「{zname}」（已穿实验服）");
                        self.try_fire(
                            Some(zid),
                            "enter",
                            tid,
                            &detail,
                            2,
                            &now,
                            frame_ts,
                            &mut out,
                        );
                    }
                    PpeStatus::NoPpe => {
                        let detail = format!("进入安全区「{zname}」未穿实验服");
                        if !self.try_fire(
                            Some(zid),
                            "enter_no_ppe",
                            tid,
                            &detail,
                            3,
                            &now,
                            frame_ts,
                            &mut out,
                        ) {
                            // 规则未配置 enter_no_ppe 时回退 enter（与 Python 合并语义一致）
                            let detail = format!("进入安全区「{zname}」（未穿实验服）");
                            self.try_fire(
                                Some(zid),
                                "enter",
                                tid,
                                &detail,
                                2,
                                &now,
                                frame_ts,
                                &mut out,
                            );
                        }
                    }
                    PpeStatus::Unknown => {
                        let detail = format!("进入安全区「{zname}」");
                        self.try_fire(
                            Some(zid),
                            "enter",
                            tid,
                            &detail,
                            2,
                            &now,
                            frame_ts,
                            &mut out,
                        );
                        let detail = format!("进入安全区「{zname}」（实验服检测未启用）");
                        self.try_fire(
                            Some(zid),
                            "enter_no_ppe",
                            tid,
                            &detail,
                            2,
                            &now,
                            frame_ts,
                            &mut out,
                        );
                    }
                },
            }
        }

        // 状态回收：保留本帧活跃轨迹（在场项即使暂时消失也保留，等离场去抖完成）
        let ids: Vec<i64> = active_ids.iter().copied().collect();
        for st in self.zone_states.values_mut() {
            st.retain(&ids, None);
        }

        // 在场人数超限：按规则 zone_id 口径统计
        let occ_rules: Vec<RuleSpec> = self
            .rules
            .iter()
            .filter(|r| r.enabled && r.behavior == "occupancy_over")
            .cloned()
            .collect();
        let any_zone: HashSet<i64> = occ.values().flatten().copied().collect();
        for rule in occ_rules {
            let zid = if rule.zone_id == 0 {
                None
            } else {
                Some(rule.zone_id)
            };
            let n = match zid {
                None => any_zone.len(),
                Some(z) => occ.get(&z).map(|s| s.len()).unwrap_or(0),
            };
            if rule.threshold > 0 && n as i64 > rule.threshold {
                let where_ = zid
                    .map(|z| format!("「{}」", self.zone_name(z)))
                    .unwrap_or_else(|| "画面".to_string());
                let detail = format!("{where_}在场人数 {n} 超过阈值 {}", rule.threshold);
                self.try_fire(
                    zid,
                    "occupancy_over",
                    0,
                    &detail,
                    rule.level,
                    &now,
                    frame_ts,
                    &mut out,
                );
            }
        }
        out
    }

    fn zone_name(&self, zid: i64) -> String {
        self.zones
            .iter()
            .find(|z| z.id == zid)
            .map(|z| z.name.clone())
            .unwrap_or_else(|| zid.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn try_fire(
        &mut self,
        zone_id: Option<i64>,
        behavior: &str,
        track_id: i64,
        detail: &str,
        level_hint: i64,
        now: &DateTime<Local>,
        frame_ts: &str,
        out: &mut Vec<Candidate>,
    ) -> bool {
        let chosen: Option<RuleSpec> = self
            .rules
            .iter()
            .find(|r| {
                r.enabled
                    && r.behavior == behavior
                    && rule_zone_ok(r, behavior, zone_id)
                    && weekday_allowed(&r.weekdays, now)
                    && self.holiday_allowed(r, now)
                    && hour_in_window(&r.time_start, &r.time_end, now)
            })
            .cloned();
        let Some(rule) = chosen else {
            return false;
        };
        let key = (rule.id, track_id);
        if let Some(last) = self.last_fire.get(&key).copied() {
            if self.frame_index.saturating_sub(last) < rule.cooldown_s.max(0) as u64 {
                return false;
            }
        }
        self.last_fire.insert(key, self.frame_index);
        out.push(Candidate {
            camera_id: self.camera.id,
            zone_id,
            behavior: behavior.to_string(),
            level: if rule.level > 0 {
                rule.level
            } else {
                level_hint
            },
            track_id,
            detail: detail.to_string(),
            ts_iso: frame_ts.to_string(),
            evidence_key: format!("{}:{}:{}", self.camera.id, frame_ts, self.frame_index),
        });
        true
    }

    fn holiday_allowed(&self, rule: &RuleSpec, now: &DateTime<Local>) -> bool {
        if !rule.holiday_only {
            return true;
        }
        let day = now.format("%Y-%m-%d").to_string();
        self.calendar
            .get(&day)
            .map(|v| v == "holiday")
            .unwrap_or(false)
    }
}

fn rule_zone_ok(rule: &RuleSpec, behavior: &str, zone_id: Option<i64>) -> bool {
    if behavior == "occupancy_over" {
        return match zone_id {
            Some(z) => rule.zone_id == 0 || rule.zone_id == z,
            None => rule.zone_id == 0,
        };
    }
    rule.zone_id == 0 || zone_id == Some(rule.zone_id)
}

/// "1-5" / "1,2,3" / 空=全部；非法串按不匹配（fail-closed，与 Python 口径一致）。
fn weekday_allowed(raw: &str, now: &DateTime<Local>) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return true;
    }
    let today = now.weekday().number_from_monday() as i64;
    for part in raw.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some((a, b)) = p.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<i64>(), b.trim().parse::<i64>()) {
                if (a..=b).contains(&today) {
                    return true;
                }
            }
        } else if p.parse::<i64>().ok() == Some(today) {
            return true;
        }
    }
    false
}

fn hour_in_window(start: &str, end: &str, now: &DateTime<Local>) -> bool {
    let hhmm = now.format("%H:%M").to_string();
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() || end.is_empty() {
        return true;
    }
    if start <= end {
        start <= hhmm.as_str() && hhmm.as_str() <= end
    } else {
        hhmm.as_str() >= start || hhmm.as_str() <= end // 跨午夜
    }
}

fn parse_ts(ts: &str) -> Option<DateTime<Local>> {
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(ts, fmt) {
            if let Some(dt) = Local.from_local_datetime(&n).single() {
                return Some(dt);
            }
        }
    }
    None
}

/// 确定性帧时间（与 Python `_det_dt` 同基准 2026-01-01 + 帧序号换算的毫秒）。
/// 注：ffmpeg rawvideo 管道不携带 PTS，Rust 侧按分析帧率等间隔近似（L1 灰度对比已足够；
/// 逐帧 PTS 对齐在 T-325 接入解码层时补齐）。
pub fn frame_ts_for(index: u64, analysis_fps: f32) -> String {
    let fps = if analysis_fps > 0.0 {
        analysis_fps as f64
    } else {
        5.0
    };
    let base = NaiveDateTime::parse_from_str("2026-01-01T00:00:00", "%Y-%m-%dT%H:%M:%S")
        .expect("固定基准时间合法");
    let dt = base + chrono::TimeDelta::milliseconds(((index as f64 / fps) * 1000.0) as i64);
    dt.format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> CameraSpec {
        CameraSpec {
            id: 1,
            name: "cam".into(),
            source: "x.mp4".into(),
            type_: "file".into(),
            analysis_fps: 5.0,
            file_loop: false,
        }
    }

    fn zone() -> ZoneSpec {
        ZoneSpec {
            id: 10,
            camera_id: 1,
            name: "A 区".into(),
            points: vec![(0, 0), (1000, 0), (1000, 1000), (0, 1000)],
        }
    }

    fn rule(id: i64, behavior: &str, level: i64, cooldown: i64) -> RuleSpec {
        RuleSpec {
            id,
            camera_id: 1,
            zone_id: 10,
            behavior: behavior.into(),
            level,
            cooldown_s: cooldown,
            time_start: "00:00".into(),
            time_end: "23:59".into(),
            weekdays: String::new(),
            threshold: 0,
            enabled: true,
            holiday_only: false,
        }
    }

    fn det(box_: [f32; 4]) -> PersonDetection {
        PersonDetection {
            box_,
            score: 0.9,
            ppe: PpeStatus::Unknown,
        }
    }

    #[test]
    fn enter_and_exit_respect_hysteresis() {
        let mut p = CameraPipeline::new(
            camera(),
            vec![zone()],
            vec![
                rule(1, "enter", 2, 0),
                rule(2, "exit", 2, 0),
                rule(3, "enter_no_ppe", 2, 0),
            ],
            HashMap::new(),
            2,
            3,
        );
        let ts = "2026-01-01T10:00:00";
        let d = [det([10.0, 10.0, 100.0, 200.0])];
        // 第 1 帧：迟滞未达阈值，不产事件
        assert!(p.process(&d, ts).iter().all(|c| c.behavior != "enter"));
        let ev1 = p.process(&d, ts);
        assert_eq!(ev1.iter().filter(|c| c.behavior == "enter").count(), 1);
        // 无检出 2 帧：仍未达 exit 阈值（3）
        assert!(p.process(&[], ts).is_empty());
        assert!(p.process(&[], ts).is_empty());
        let ev3 = p.process(&[], ts);
        assert_eq!(ev3.len(), 1);
        assert_eq!(ev3[0].behavior, "exit");
        assert_eq!(ev3[0].detail, "离开安全区「A 区」");
    }

    #[test]
    fn occupancy_over_uses_unique_tracks_and_cooldown() {
        let mut p = CameraPipeline::new(
            camera(),
            vec![zone()],
            vec![RuleSpec {
                threshold: 2,
                level: 3,
                ..rule(9, "occupancy_over", 3, 5)
            }],
            HashMap::new(),
            2,
            3,
        );
        let ts = "2026-01-01T10:00:00";
        let three = [
            det([0.0, 0.0, 10.0, 100.0]),
            det([100.0, 0.0, 110.0, 100.0]),
            det([200.0, 0.0, 210.0, 100.0]),
        ];
        let first = p.process(&three, ts);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].behavior, "occupancy_over");
        assert!(first[0].detail.contains("在场人数 3 超过阈值 2"));
        // 冷却期内（5 帧）不再触发
        for _ in 0..4 {
            assert!(p.process(&three, ts).is_empty());
        }
        assert_eq!(p.process(&three, ts).len(), 1);
    }

    #[test]
    fn time_window_and_weekday_filter() {
        let make = || {
            CameraPipeline::new(
                camera(),
                vec![zone()],
                vec![RuleSpec {
                    time_start: "08:00".into(),
                    time_end: "18:00".into(),
                    weekdays: "1-5".into(),
                    ..rule(1, "enter", 2, 0)
                }],
                HashMap::new(),
                1,
                1,
            )
        };
        let d = [det([10.0, 10.0, 100.0, 200.0])];
        // 2026-01-03 是周六 → weekday 过滤
        assert!(make().process(&d, "2026-01-03T10:00:00").is_empty());
        // 2026-01-01 周四 20:00 → 时段外
        assert!(make().process(&d, "2026-01-01T20:00:00").is_empty());
        // 2026-01-01 周四 10:00 → 命中
        assert!(!make().process(&d, "2026-01-01T10:00:00").is_empty());
    }

    #[test]
    fn no_ppe_behavior_falls_back_to_enter_without_rule() {
        let mut p = CameraPipeline::new(
            camera(),
            vec![zone()],
            vec![rule(1, "enter", 2, 0)],
            HashMap::new(),
            1,
            1,
        );
        let d = [PersonDetection {
            box_: [10.0, 10.0, 100.0, 200.0],
            score: 0.9,
            ppe: PpeStatus::NoPpe,
        }];
        let ev = p.process(&d, "2026-01-01T10:00:00");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].behavior, "enter");
        assert!(ev[0].detail.contains("未穿实验服"));
    }

    #[test]
    fn frame_ts_is_deterministic() {
        assert_eq!(frame_ts_for(0, 5.0), "2026-01-01T00:00:00");
        assert_eq!(frame_ts_for(5, 5.0), "2026-01-01T00:00:01");
        assert_eq!(frame_ts_for(10, 5.0), "2026-01-01T00:00:02");
    }
}
