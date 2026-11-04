//! ZoneState 进出迟滞：与 Python `server/app/zones.py::ZoneState`（T-306）语义逐位一致。
//! 测试向量共享：`testdata/vectors/zone_state_hysteresis.json`。

use serde::Deserialize;

use crate::ZoneChange;

/// 迟滞参数（与 Python `LS_ZONE_ENTER_FRAMES` / `LS_ZONE_EXIT_FRAMES` 对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Hysteresis {
    pub enter_frames: u32,
    pub exit_frames: u32,
}

#[derive(Clone, Copy)]
struct TrackState {
    inside: bool,
    streak: u32,
    pending: Option<bool>,
}

/// 单个区域的按-track 进出状态机。
pub struct ZoneState {
    hysteresis: Hysteresis,
    state: std::collections::HashMap<i64, TrackState>,
}

impl ZoneState {
    pub fn new(hysteresis: Hysteresis) -> Self {
        ZoneState {
            hysteresis: Hysteresis {
                enter_frames: hysteresis.enter_frames.max(1),
                exit_frames: hysteresis.exit_frames.max(1),
            },
            state: std::collections::HashMap::new(),
        }
    }

    /// 观测一帧：`Some(Enter/Exit)` = 状态翻转（等价 Python 的 (entered, exited)），
    /// `None` = 无变化。语义与 Python `ZoneState.update` 逐位一致：
    /// 观测与判定一致 → 连击清零；方向翻转 → 连击重计；连击达阈值 → 翻转。
    pub fn update(&mut self, track_id: i64, inside: bool) -> Option<ZoneChange> {
        let need = if inside {
            self.hysteresis.enter_frames
        } else {
            self.hysteresis.exit_frames
        };
        let s = self
            .state
            .entry(track_id)
            .or_insert(TrackState { inside: false, streak: 0, pending: None });
        if inside == s.inside {
            s.streak = 0;
            s.pending = None;
            return None;
        }
        if s.pending != Some(inside) {
            s.pending = Some(inside);
            s.streak = 1;
        } else {
            s.streak += 1;
        }
        if s.streak >= need {
            s.inside = inside;
            s.streak = 0;
            s.pending = None;
            return Some(if inside {
                ZoneChange::Enter
            } else {
                ZoneChange::Exit
            });
        }
        None
    }

    /// 当前判定为在区域内的 track（与 Python `inside_ids` 对齐；顺序不保证）。
    pub fn inside_ids(&self) -> Vec<i64> {
        self.state.iter().filter(|(_, s)| s.inside).map(|(k, _)| *k).collect()
    }

    /// 状态表大小（与 Python `size` 对齐）。
    pub fn size(&self) -> usize {
        self.state.len()
    }

    /// 内存回收：删除"非活跃且已判离场"的轨迹；超 cap 按插入序淘汰非活跃项。
    /// 语义与 Python `retain` 一致（在场项即使消失也保留，等待离场去抖完成）。
    pub fn retain(&mut self, active: &[i64], max_size: Option<usize>) -> usize {
        let active: std::collections::HashSet<i64> = active.iter().copied().collect();
        let mut removed = 0usize;
        let stale: Vec<i64> = self
            .state
            .iter()
            .filter(|(tid, s)| !active.contains(tid) && !s.inside)
            .map(|(tid, _)| *tid)
            .collect();
        for tid in stale {
            self.state.remove(&tid);
            removed += 1;
        }
        let cap = max_size.unwrap_or(5000).max(0);
        while self.state.len() > cap {
            // 与 Python cap 淘汰对齐：按插入序淘汰任意非 active 项（含在场待去抖项），
            // active 项绝不淘汰（保住在场/离场语义）
            let victim = self
                .state
                .iter()
                .find(|(tid, _)| !active.contains(tid))
                .map(|(tid, _)| *tid);
            match victim {
                Some(tid) => {
                    self.state.remove(&tid);
                    removed += 1;
                }
                None => break,
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_acceptance_sequence() {
        // 76 文档验收序列：[in,out,in,in,out,out,out] → 恰好 1 enter + 1 exit
        let mut st = ZoneState::new(Hysteresis { enter_frames: 2, exit_frames: 3 });
        let seq = [true, false, true, true, false, false, false];
        let got: Vec<Option<ZoneChange>> = seq.iter().map(|&v| st.update(1, v)).collect();
        let want = vec![
            None,
            None,
            None,
            Some(ZoneChange::Enter),
            None,
            None,
            Some(ZoneChange::Exit),
        ];
        assert_eq!(got, want);
        // 再进入重新去抖
        assert_eq!(st.update(1, true), None);
        assert_eq!(st.update(1, true), Some(ZoneChange::Enter));
    }

    #[test]
    fn threshold_one_matches_legacy() {
        let mut st = ZoneState::new(Hysteresis { enter_frames: 1, exit_frames: 1 });
        assert_eq!(st.update(1, true), Some(ZoneChange::Enter));
        assert_eq!(st.update(1, false), Some(ZoneChange::Exit));
        assert_eq!(st.update(1, false), None);
        assert_eq!(st.update(1, true), Some(ZoneChange::Enter));
    }

    #[test]
    fn retain_keeps_inside_drops_exited() {
        let mut st = ZoneState::new(Hysteresis { enter_frames: 1, exit_frames: 1 });
        for tid in 0..100i64 {
            st.update(tid, true);
        }
        // 全部在场 → 第一轮 stale 回收 0；cap=10 按插入序淘汰非 active 90 个
        let removed = st.retain(&[0, 1, 2], Some(10));
        assert_eq!(removed, 90);
        assert_eq!(st.size(), 10); // active 3 + cap 补足
    }
}
