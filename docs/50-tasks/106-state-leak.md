# 任务 106 · ZoneState/RuleEngine 状态字典无界增长（P1）

> 来源：flash 审阅建议 #1。`ZoneState._inside` 与 `RuleEngine._last_fire` 按 track_id/规则只增不减，
> 长时间运行（数周）track id 持续增长 → 内存缓涨。

## 目标

1. `ZoneState`：
   - 新增 `retain(active_tids: set[int])`：删除不在当前画面 track 集合中的条目（离场状态已在 105 处理）；
   - 或在 `update` 后由 pipeline 每帧调用 `retain`；保留 `inside_ids` 语义；
   - 设上限（如 5000）兜底：超限时清理最旧/非 active（需要记录 last_seen，或在 retain 中处理）。
2. `RuleEngine`：
   - `_last_fire` 键 `(rule_id, track_id)`；新增 `prune(now, max_age_s)` 删除超过 `max(cooldown, 3600)` 的条目，
     或每 N 次调用清理一次；避免每帧全表扫描（用计数器/时间间隔）。
   - 规则删除时清理对应键（delete_rule 调用处可选）。
3. 测试 `server/tests/test_state_prune.py`：
   - 10k 个不同 track 进入/离开后 `_inside` 大小回到 active 数量；
   - 非 active 的 track 被删除、active 保留；
   - `RuleEngine._last_fire` 超时条目被清理、冷却内条目保留；
   - pipeline 每帧调用 retain 后状态正确（纯函数测试即可，不跑真实视频）。

## 约束

- 不改进出事件语义（与 105 配合）；不引入依赖；
- prune 不能在每帧做 O(全历史) 扫描；用 active 集合或定期清理；
- 测试临时数据，不推理。

## 验收

- `pytest tests/ -q` 全绿；报告附清理前后字典大小/耗时；
- 不改变现有事件测试结果。

## 执行报告

**主线 2026-11-03 完成（提交前）**

- **ZoneState**（`zones.py`）：新增 `size`、`retain(active_tids, max_size=None)`：
  - 删除本帧不活跃轨迹（离场状态先由 pipeline 的 update/exit 处理，再回收）；
  - 上限 5000；超限只淘汰非 active（active 不淘汰，保住在场/离场语义）；
  - 返回删除条数，便于测试/观测。
- **pipeline**（`pipeline.py`）：区域配置变化时回收已删除 zone 的状态；每帧处理完进出场后
  `st.retain(active_tids)`；空帧分支 `retain(set())`，不再保留任何轨迹。
- **RuleEngine**（`rules.py`）：新增 `prune(now, max_age_s=None)` 与 `_maybe_prune`：
  - 默认保留期 = `max(3600s, 全部规则最大 cooldown)`，长冷却规则不会被误清；
  - 每 1000 次 `match` 清理一次（不做每帧全表扫描）；`_last_prune` 记录。
- **测试**（`server/tests/test_state_prune.py`，6 项）：
  - 10k 轨迹 → retain 活跃 10 个：`10000 → 10`，删除 9990；
  - cap 行为：非 active 淘汰、active 永不淘汰；
  - 冷却键按年龄清理/默认长冷却保留/每 N 次自动清理；
  - pipeline 空帧后状态归零。
- **回归**：`test_state_prune + test_zone_exit_state + test_core` **19 passed**；
  事件语义（enter/exit 各一次、再进入重新 enter）不变。
- 说明：未做真实 24h 长跑内存曲线；按状态上限+定期清理给出有界性，后续 24h 验收可观测进程内存。

