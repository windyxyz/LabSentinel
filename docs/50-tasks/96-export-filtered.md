# 任务 96 · 按当前筛选导出事件（P2）

> 背景：Events 页能筛选/搜索（84 完成后），但导出只能按天数/摄像头，用户想“把当前筛选结果导出”
> 还得手动再筛。本任务让 CSV 导出复用全部筛选条件，并在 Events 页提供按钮。

## 目标

### 1. 后端导出扩展

- `GET /api/export/events.csv` 增加参数：`level`、`start_ts`、`end_ts`、`q`（与 events 查询同语义、
  同 LIKE 转义）；保留 `days`/`camera_id`/`status` 兼容：
  - 未给 `start_ts` 时用 `days` 计算窗口；给了 `start_ts` 则以显式区间为准；
  - 行内容/列/BOM/流式保持现状；空结果输出仅表头（不报错）。
- `GET /api/export/evidence.zip` 增加 `q` 参数（现有 camera/level/status/start/end 保留），
  README 中记录筛选条件；空结果 → 仍生成 zip（README 说明无事件）还是 400？定义为 400「无匹配事件」，
  文档写明（与现有 400 超限一致）。
- 不新增端点；不改变默认行为（不传新参数时与现在一致）。

### 2. 前端 `web/src/views/Events.vue`

- 工具栏增加「导出当前筛选 CSV」按钮：用当前 camId/level/status/start/end/q 拼 URL 下载
  （`<a download>` 或 `window.location`），文件名带日期；
- 若当前无筛选，提示“将导出最近 30 天全部事件”（文档口径）；
- 不改 Reports 现有导出按钮；不改全局四件套。

### 3. 测试 `server/tests/test_export_filtered.py`

- 造事件（不同 level/status/日期/detail）：
  - q=实验服 + level=3 → CSV 行数与 events/count 一致；
  - start/end 显式区间优先于 days；
  - 旧参数（days/camera/status）回归；
  - 空结果仅表头；
  - evidence.zip 带 q 过滤后事件数正确、README 含筛选说明；无匹配 400；
  - 中文/逗号/引号详情 CSV 转义正确（现有导出逻辑）。

### 4. 文档

- `docs/35-api-reference.md` 导出参数表更新；
- `docs/运维手册.md` 报表/导出一节；
- `docs/部署FAQ.md` 可选。

## 约束

- 不改现有 CSV 列顺序/编码；不引入依赖；
- 大文件仍流式（不能因加 q 变成全量内存拼接）；
- 前端只改 Events.vue。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 实跑：筛选 q+level 导出，行数与页面计数一致；打开 CSV 中文正常（BOM）；
- 报告附行数对照。

## 执行报告

（待填：改动文件 / 导出行数对照 / 测试与 build / 遗留）
