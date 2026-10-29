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

（待填：改动文件 / zip 证据 / 测试与 build / 遗留）
