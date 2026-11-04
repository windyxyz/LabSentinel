# 任务 132 · AI 复核徽标与筛选（P2）

> VLM 预分类（124）已写 `events.ai_review/ai_verdict`，但前端事件页完全不显示；本任务让结果可见、
> 可筛，并保持“仅供参考、不改处置状态”的语义。

## 目标

1. 后端事件查询增加 `ai_verdict` 过滤（`likely_true|likely_false|uncertain|none`）：
   - `GET /api/events?ai_verdict=likely_false`、`/api/events/count` 同步；
   - `none` = `ai_review IS NULL`；其它 = `json_extract(ai_review,'$.verdict')=?`（JSON1；
     若不可用退回 LIKE，报告注明）；
   - 不影响默认查询/现有筛选组合；参数非法 400。
2. `web/src/views/Events.vue`：
   - 表格类型旁/详情抽屉显示徽标：`AI: 疑似误报`（橙）/`疑似真实`（绿）/`不确定`（灰）；
     `title` 显示 reason/confidence/model；
   - 筛选栏加“AI 复核”下拉（全部/疑似误报/疑似真实/不确定/未复核）；
   - 徽标只读，不改变处置按钮/状态。
3. `Reports.vue` 处置质量卡增加 `by_ai_verdict` 小计（已有 API 字段，仅展示）。
4. 旧事件（ai_review NULL）不显示徽标、筛选“未复核”可见。

## 测试/验收

- `pytest`：store/API 过滤（4 类 + 组合 camera/status）、非法值 400、count 与列表一致；
  旧库 NULL 兼容。
- `npm run build`；Events 页手工/脚本验证：造 2 条不同 verdict 事件 → 筛选各 1 条、徽标文案正确。
- 报告附 API 样例 JSON 与 build gzip。

## 约束

- 不改 ai_review 写入/不自动改 status；未启用 VLM 时页面无额外请求（筛选默认全部）。
- 不新增生产依赖。
