# 任务 103 · 单事件证据包导出（P2）

> 背景：已有批量证据包（任务 33），但处理单条事件时希望一键拿到“事件 JSON + 截图 + 短视频”，
> 发给主管/供应商。本任务提供单条 zip，路径安全，缺文件也能出包。

## 目标

### 1. `GET /api/events/{eid}/evidence.zip`

- 事件不存在 → 404；
- zip 内容：
  - `event.json`：事件全字段（id/ts/camera_id/type/level/track_id/detail/status/handled_by/handled_ts）、
    摄像头名称（join，不存在则 null）、导出时间；
  - `snapshot.jpg`：若 `snapshot` 非空、路径在 `settings.data_dir` 内（用 76 的严格包含判断）、文件存在；
  - `video.mp4`：同上规则（video 列）；
  - `README.txt`：说明字段/缺失文件原因/生成器版本；
- 文件缺失/越界 → 跳过并在 README 写明，不 500；
- zip 内存生成（`io.BytesIO`），总大小上限 200MB（超过 400 并提示用批量导出）；
- `Content-Disposition: attachment; filename="event-<id>-evidence.zip"`；
- 读保护口径同事件读取（默认公开；`auth_read_protected` 时 require_read）。

### 2. 前端 `web/src/views/Events.vue`

- 详情抽屉底部加「下载证据包」按钮（`<a :href>` download）；
- 无截图/视频时不置灰（仍可下 JSON+README），文案提示“无媒体时仅含事件信息”；
- 只改 Events.vue，不改全局四件套。

### 3. 测试 `server/tests/test_event_evidence.py`

- 事件含有效截图+视频 → zip 条目齐全、event.json 字段正确、README 存在；
- 只有截图/都没有 → 对应条目缺失、README 有说明、仍 200；
- snapshot 指向 data 外/`..` → 跳过（不打包）；
- 事件不存在 404；超大文件（mock stat/构造）→ 400；
- 鉴权口径。

### 4. 文档

- `docs/35-api-reference.md` 端点；
- `docs/运维手册.md` 事件处置/证据；
- `docs/部署FAQ.md` 可选。

## 约束

- 路径安全优先（76 完成后复用其函数）；不打包 data 外文件；
- 不新增依赖；不改变批量证据包行为；
- 前端只改 Events.vue。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库：造一条带截图事件，下载 zip 解压检查条目/JSON；
- 报告附 zip 清单与大小。

## 执行报告

**flash 2026-11-04 完成（pytest 450 passed = 445+5 新增；build gzip 60.11KB；8188 实机 PASS）**
（全量回归为 117 基线上 103 的口径，结果见文末）

### 1. 后端（export.py `build_event_evidence` + api.py 路由）

- 事件不存在 → None（404）；zip 内存生成（io.BytesIO）：`event.json`（全字段+camera_name
  join+exported_at）+ `snapshot.jpg` + `video.mp4` + `README.txt`（生成信息/字段说明/
  **缺失条目逐条写明原因**/合规提示）；
- 路径安全复用 76 的 `_within_data_dir` 严格包含（`..` 穿越/跨盘/data 外一律跳过不打包）；
  文件不存在（保留策略清理后）跳过写明——**永不 500**；
- 总量 >200MB 抛 ValueError（400，提示改用批量 `/api/export/evidence.zip`）；
- `Content-Disposition: attachment; filename="event-<id>-evidence.zip"` + `X-Evidence-Files`；
  读保护 `require_read`（默认公开、auth_read_protected 时需令牌，口径同事件读取）；
- 批量证据包行为零改动。

### 2. 前端（Events.vue，路由零新增）

详情抽屉底部「下载证据包」按钮（`<a download>` 直链）；无媒体不置灰，
文案提示"无媒体时仅含事件信息与说明 README"。

### 3. 测试（test_event_evidence.py，5 用例）

全条目包（zip 清单/event.json 字段/媒体字节）、无媒体与越界（`..` 穿越/跨盘/不存在 →
README 写明、仍出包）、事件不存在 None、超大（mock getsize → ValueError 提示批量）、
API 全链（headers/文件名/404）、鉴权 401。
**坑记录**：①heredoc `\\n` 二度伤文件（export.py README join 断行）——后续一律 Edit 工具；
②实机断言大小写敏感取头（uvicorn 小写 `content-disposition`）——检查脚本改 lower。

### 4. 实机验证（8188 临时实例）

造带截图事件 → GET zip：`entries=['event.json','snapshot.jpg','README.txt']`（视频缺失
按设计跳过）、event.json camera_name=e103/exported_at 在、README 三行头正确、
Content-Disposition 文件名正确、不存在事件 404 → **PASS**。

### 5. 文档

35-api-reference 新端点小节；运维手册 §6.1 单事件证据分发。

### 遗留

- 无。**Phase B 全部完成**（112/111/113/116/115/117/103）。
