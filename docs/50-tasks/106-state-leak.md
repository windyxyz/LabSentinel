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

（待填：改动 / 大小对比 / 测试 / 遗留）
