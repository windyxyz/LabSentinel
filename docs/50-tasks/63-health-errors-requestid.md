# 任务 63 · 健康探针 + 统一错误响应 + 请求 ID

> 背景：服务端目前只有 `/api/system`（依赖数据库/引擎），没有轻量存活/就绪探针；未捕获异常会返回
> uvicorn 默认 500 文本，排障时无法把前端报错、服务端堆栈、日志对应起来。本任务补三项基础设施，
> 为 61 的诊断包、59 的冒烟脚本、运维监控提供稳定接口。

## 目标

### 1. 健康探针（无需鉴权，永远不触发推理）

- `GET /api/health/live`：进程存活即 `200 {"status":"ok"}`；不查 DB、不读模型。
- `GET /api/health/ready`：逐项检查并返回结构化结果；全部通过 200，否则 **503**：
  ```json
  {"status":"ready|degraded","checks":{
    "db":{"ok":true,"detail":"..."},
    "data_dir":{"ok":true,"detail":"可写"},
    "model":{"ok":true,"detail":"..."},
    "disk":{"ok":true,"free_gb":12.3}}}
  ```
  - db：`SELECT 1`；data_dir：写一个临时文件再删；model：`settings.ppe_model` 或 `model_fp32` 存在；
    disk：`shutil.disk_usage(settings.data_dir).free` ≥ 1GB（阈值可配 `LS_READY_MIN_FREE_GB`，默认 1）。
  - **不要**加载 ONNX 会话（避免 DML 并发/启动变慢）。

### 2. 请求 ID 中间件

- 读取请求头 `X-Request-ID`（合法长度 1–128，否则忽略）或生成 `uuid4().hex`；
- 响应头始终带 `X-Request-ID`；
- 用 `contextvars` 保存当前请求 ID，日志格式可选用（见任务 61；若 61 未完成则只做响应头/错误体）；
- 未捕获异常日志必须包含 request_id 与路径。

### 3. 统一错误响应

- 保留现有 `detail` 字段（前端 `api.js` 依赖），新增 `request_id`、`code`：
  - `HTTPException` → 原状态码，`{"detail": ..., "request_id": "...", "code": "http_<status>"}`；
  - `RequestValidationError` → 422，`detail` 为首个错误的可读摘要（不要整段 pydantic 结构），
    `code="validation_error"`；
  - 未捕获 `Exception` → 500，`{"detail":"服务器内部错误", "request_id":"...", "code":"internal_error"}`，
    堆栈只进日志，**不回传**内部路径/密钥。
- 用 `@app.exception_handler` 实现；不要改变成功响应结构。

### 4. 测试 `server/tests/test_health_errors.py`

- live 200；ready 在临时 data_dir/model 存在时 200、删模型后 503 且 `checks.model.ok=false`；
- 请求 ID：客户端传入被回显；不传则生成且格式合法；响应头存在；
- 404/422/500 三种错误体都含 `detail` + `request_id` + `code`；500 不含 traceback/绝对路径；
- 现有测试不回归（`pytest tests/ -q` 基线 91 + 新增）。

### 5. 文档

- `docs/35-api-reference.md` 增加 health 两个端点与错误响应约定；
- `docs/部署FAQ.md` 增加「怎么判断服务是否就绪 / 报错里的 request_id 怎么查日志」。

## 约束

- 不改任何业务端点路径/成功响应；不改 `theme.css/style.css/App.vue/main.js`；
- 健康探针不触发 DML 推理；测试用临时目录，不碰真实 `server/data`；
- 若 61 已完成，复用其日志 request_id 过滤器；若未完成，不阻塞，只做响应头与错误体。

## 验收标准

- `pytest tests/ -q` 全绿；8001 临时实例 curl/httpx 实跑 live/ready/404/422 各一次；
- 响应头/错误体包含 request_id；日志中能用该 id 找到对应错误行（61 完成后）；
- 报告附实际 JSON（截断到合理长度）。

## 执行报告

**flash 2026-10-30 完成（pytest 122 passed，8001 实跑四类响应全过）**

- **改动文件**：`api.py` 加 `GET /api/health/live|ready`（live 不碰 DB/模型；ready 逐项 db select 1/
  data_dir 写删探针/模型文件存在/磁盘≥1GB（`LS_READY_MIN_FREE_GB` 新 settings 字段），503 语义，
  **不加载 ONNX**）；`main.py` 加请求 ID 中间件（X-Request-ID 1–128 回显否则 uuid4.hex，contextvars
  保存）+ 三个异常处理器（StarletteHTTPException→原码 `http_<status>`、RequestValidationError→422
  首错可读摘要 `validation_error`、Exception→500 固定文案 `internal_error`，堆栈只进日志）+ 中间件层
  兜底；`logging_setup.py` 格式升级为 `[req=%(request_id)s]`（过滤器从 contextvars 注入，ring/文件
  handler 共用）。`config.py` 加 `ready_min_free_gb`。
- **测试**：`test_health_errors.py` 5 用例（live/ready 全过→删模型引用 503 且 model.ok=false/
  请求 ID 回显·生成·超长忽略/404·422 错误体三字段齐/500 体不含内部异常文本+caplog 断言日志含
  request_id 与路径）。全量 **pytest 122 passed**（117 + 5）。
- **8001 实跑**：live `{"status":"ok"}`；ready 200 四项全过（model 指向 yolo11n_ppe.onnx）；
  404 `{"detail":"Not Found","request_id":"d883…","code":"http_404"}`；422
  `{"detail":"body.name: Field required","code":"validation_error"}`。
- **发现并修复的真问题**：未捕获异常日志最初写 `uvicorn.error`——该 logger **不向 root 传播**，
  server.log 里永远检索不到 `[req=...]`（实跑验证为空）。已改 `app.errors` logger（向 root 传播 →
  落 server.log），测试 caplog 断言钉死该行为。
- **文档**：35-api-reference 新增 §8.1（探针/请求 ID/错误体约定）；部署FAQ 加「怎么判断服务是否
  就绪 / request_id 怎么查日志」。
- **遗留**：无。
