# 任务 101 · OpenAPI 快照与示例（P2）

> 背景：前端/第三方对接靠 `docs/35-api-reference.md` 手写文档，端点增多后容易漂移。本任务导出
> `docs/openapi.json` 快照，并给关键端点补示例，作为机器可读契约。

## 目标

### 1. `scripts/export_openapi.py`

```
python scripts/export_openapi.py [--out docs/openapi.json] [--check]
```

- 导入 `app.main:app`（不启动 lifespan/不改 DB），`json.dump(app.openapi(), ensure_ascii=False, indent=2)`；
- `--check`：生成到临时文件与现有 `docs/openapi.json` 比较，不一致 exit 2（CI/RC 用）；
- 输出路径默认 `docs/openapi.json`；失败给可读错误。

### 2. 关键端点示例（`server/app/models.py` 或路由 `openapi_extra`）

- 至少：`POST /api/cameras`、`GET /api/cameras/{cid}/status`、`GET /api/health/ready`、
  `GET /api/events`、错误响应（63 的 `{detail,request_id,code}`）、`POST /api/events/bulk-status`（83 后）。
- 示例值不得含真实密钥/地址；用 `rtsp://user:pass@192.168.1.10/...` 明确是示例。
- 不要求所有端点都有示例，覆盖核心即可。

### 3. 测试 `server/tests/test_openapi.py`

- `app.openapi()` 可序列化；`paths` 含 health/events/cameras/usb-devices/diagnostics；
- 示例 JSON 可解析、无 `LS_ADMIN_TOKEN` 等敏感键；
- `schema_version`/`version` 字段存在；
- `--check` 对一致文件 0、对篡改文件 2（用临时目录）。

### 4. 文档

- `docs/35-api-reference.md` 顶部加“机器可读契约：docs/openapi.json（由脚本生成，勿手改）”；
- `scripts/README.md` 加命令；
- `docs/发布核对报告.md` 的自动项可加 `export_openapi --check`（可选）。

## 约束

- 不启动服务/DB/模型；不引入依赖；
- 快照文件较大可接受；不包含密钥；
- 不改业务端点行为。

## 验收

- `pytest tests/ -q` 全绿；`python scripts/export_openapi.py --check` 对当前仓库 0；
- `docs/openapi.json` 可被 `json.load`；报告附路径数/大小。

## 执行报告

（待填：改动文件 / 快照统计 / 测试 / 遗留）

## 执行报告

**flash 2026-11-04 完成（pytest 491 passed = 488+3 新增；--check 对当前仓库 0）**

### 1. scripts/export_openapi.py（新文件）

- `build_openapi()`：只导入 app.main:app（不跑 lifespan/不碰 DB/模型）→ app.openapi() →
  **脚本侧集中注入关键端点示例**（POST /api/cameras 请求体、GET cameras/{cid}/status、
  health/ready、events 200、bulk-status 请求、evidence.zip 404 错误形状
  `{detail,request_id,code}`——示例值全演示数据 `rtsp://user:pass@192.168.1.10/...`）；
- `--check`：temp 文件（**同尾换行**——初版 temp 与文件差 1 字节恒 FAIL，已修）比对，
  不一致 exit 2；输出 `docs/openapi.json`（**85 paths / 102KB**）。

### 2. 测试（test_openapi.py，3 用例）

可序列化+9 条关键路径存在（含 124/125/126 新端点）、示例注入+敏感键零命中、
--check 一致 0/篡改 2（真实快照备份-篡改-还原，全程 json.load 可读）。

### 3. 文档

35-api-reference 顶部"机器可读契约"指引；scripts/README 命令。

### 遗留

- 无。release_check 集成 `export_openapi --check` 项随 129 发布卫生一起做。
