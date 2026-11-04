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

## 执行报告

**flash 2026-11-04 完成（pytest 499 passed = 495+4 新增；build gzip 62.12KB；8188 实机 PASS）**
（全量回归为 131 基线上 132 的口径，数字以本轮实跑为准）

### 1. 后端（store + api）

- `Store._ai_clause`：`none` → `ai_review IS NULL`；其它 →
  `json_extract(ai_review,'$.verdict')=?`（JSON1 内建，无需回退 LIKE）；`events()` 与
  `count_events()` 同步加 `ai_verdict` 参——与 camera/status/q 组合可叠加；
- `GET /api/events` / `/api/events/count` 加 `ai_verdict` 查询参数；
  `_check_ai_verdict`：非法值（含空串/注入形态）400，合法集
  likely_true|likely_false|uncertain|none；
- 默认查询/现有筛选零影响（不传参不过滤）。

### 2. 前端

- **Events.vue**：筛选栏新增「AI 复核」下拉（全部/疑似误报/疑似真实/不确定/未复核，
  change 即 resetAndLoad）；表格类型列旁徽标（疑似误报=橙/疑似真实=绿/不确定=灰），
  title 含 reason/confidence/model；详情抽屉标题旁同徽标 +「（仅供参考）」；
  徽标纯展示，处置按钮/状态零改动；旧事件（NULL）无徽标、筛选"未复核"可见；
  CSS 三类 scoped；
- **Reports.vue**：处置质量卡下加 `by_ai_verdict` 小计行（复用 102 已有 API 字段）；
- 未启用 VLM 时无额外请求（筛选默认全部，仅既有 events 拉取）。

### 3. 测试（test_ai_filter.py，4 用例）

store 四类过滤互斥完备（并集=全量）、count 一致、uncertain×status 组合、camera 组合、
默认查询不变；API likely_false/none 过滤+count、`ai_review` 对象与 `ai_verdict` 回显、
非法值 400（含注入形态）、旧库 NULL 迁移兼容。

### 4. 实机验证（8188 临时实例）

造 2 条（likely_false/未复核）：`?ai_verdict=likely_false` → [e1]、`none` → [e2]，
count 各 1，`ai_verdict/ai_review.reason` 回显正确，非法参数 400 → **PASS**。

### 遗留

- 无。
