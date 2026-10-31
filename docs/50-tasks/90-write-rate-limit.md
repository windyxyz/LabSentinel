# 任务 90 · 写接口简单限流（P2）

> 背景：如果服务暴露到办公网，脚本/误操作可能高频写库（添加/删除摄像头、批量处置），造成锁竞争或
> 数据事故。本任务给写接口加轻量滑动窗口限流；读接口不受影响。

## 目标

### 1. 中间件/依赖

- 配置：
  - `LS_WRITE_RATE_LIMIT=120`（每分钟允许的写请求数；0=关闭）
  - `LS_WRITE_RATE_WINDOW_S=60`
  - `LS_TRUST_PROXY=0`（1 时才信任 `X-Forwarded-For` 第一跳，否则用 `request.client.host`）
- 仅对 `POST/PUT/PATCH/DELETE` 且 path 以 `/api/` 开头；**排除** `/api/health/*`（无写）与
  `/api/alert-test`？测试/自检也计入（避免滥用），但文档说明。
- 超限 → `429`，JSON `{detail:"写操作过于频繁，请稍后重试", request_id, code:"rate_limited"}`，
  `Retry-After` 秒；`X-Request-ID` 仍在。
- 内存实现：`dict[key] = deque[timestamps]`，线程锁；每次清理过期项 + 定期删除空 key；
  key = `client_ip`（不做 per-path，简单可预期）。
- 中间件顺序：在请求 ID 之后、业务之前；不能影响 GET/WS。
- 测试可通过 `LS_WRITE_RATE_LIMIT=2` 构造。

### 2. 测试 `server/tests/test_rate_limit.py`

- 限制 2/min：同一 client 第 3 个 POST → 429 + Retry-After；第 1/2 正常；
- GET 连续 10 次不受限；超过窗口后可再写（monkeypatch time 或窗口设 1s）；
- 不同 client（伪造 client host / TestClient 不同 IP 不可行时用函数单测）互不影响；
- `X-Forwarded-For` 在 `trust_proxy=0` 时被忽略，=1 时生效；
- 429 响应带 request_id；0=关闭。

### 3. 前端

- `api.js` 已有错误提示；429 时显示 detail 即可，无需改动（若需，加友好前缀“操作过快：”）。

### 4. 文档

- `.env.example` 3 项注释；
- `docs/部署指南.md` 反向代理/安全一节；
- `docs/部署FAQ.md`「提示写操作过于频繁怎么办 / 反向代理后所有请求同一 IP」。

## 约束

- 内存限流（单进程）即可，不引入 Redis；重启清零（文档说明）；
- 不影响健康探针/静态资源/WS；不改变成功响应；
- 默认 120/min 足够单人操作，不误伤正常使用。

## 验收

- `pytest tests/ -q` 全绿；
- 8001 实跑 limit=2：3 次 POST 第 3 次 429；GET 不受影响；
- 报告附状态码序列。

## 执行报告

**flash 2026-10-31 完成（pytest 258 passed、5 用例全过、429 实测）**

- **改动**：`config.py` 加 `write_rate_limit=120`/`write_rate_window_s=60`/`trust_proxy=False`；
  `main.py` 限流逻辑合并进 `request_id_middleware`（避免独立中间件栈导致 TaskGroup 冲突——
  实测独立 `@app.middleware("http")` 返回 JSONResponse 会撞 "unhandled errors in a TaskGroup"）。
  滑动窗口 dict[ip]→deque[timestamps] + `_rate_lock`；超限 → 429 JSON + Retry-After +
  X-Request-ID；`LS_TRUST_PROXY=1` 时取 X-Forwarded-For 第一跳；定期清空 key 防内存增长。
- **测试**（test_rate_limit.py 5 用例）：limit=2 第 3 个 POST 429 + Retry-After + request_id、
  GET 不受限；窗口过期可再写（monkeypatch time）；trust_proxy 开关行为；0=关闭；429 带
  request_id。全量 **pytest 258 passed**（253 + 5）。
- **文档**：.env.example 3 项注释；部署指南反代/安全段；部署FAQ「写操作频繁/反代同 IP」。
- **遗留**：单进程内存限流（重启清零），多 worker 部署需前置限流（记入运维文档）。
