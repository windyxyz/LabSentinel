# 任务 77 · gzip 大响应保护（P1）

> 背景：任务 72 的选择性 gzip 中间件把整个响应体读进内存再压缩。当前资产/JSON 很小没问题，但
> `text/csv` 事件导出可能到几十 MB 甚至更大，全部缓冲会推高内存；`StreamingResponse` 也没有
> Content-Length 可判断。本任务加保护：**只压缩已知大小且不超过阈值的小响应，流式/大响应直接放行**。

## 目标

- `server/app/main.py` 的 `selective_gzip_middleware`：
  - 读取 `Content-Length`；若存在且 > `LS_GZIP_MAX_BYTES`（默认 4MB）→ 不压缩，原样返回；
  - 若不存在（chunked/streaming）→ 不压缩（避免缓冲未知大小流）；
  - 小于阈值才读 body + gzip；
  - 保留现有 `Vary`、Range 让路、类型白名单、304 行为；
  - 对 `Content-Length` 缺失但 body 已完全可读的普通 `Response`（Starlette 通常有 length）可压缩；
    实现上用 `resp.headers.get("content-length")` 判断即可。
- 配置 `server/app/config.py`：`gzip_max_bytes: int = 4 * 1024 * 1024`；`.env.example` 注释。
- 测试 `server/tests/test_web_static.py`（或新 `test_gzip_guard.py`）：
  - 小 JSON（<阈值）→ gzip，解压一致；
  - 大文本（构造 > 阈值，Content-Length 已知）→ **不 gzip**，body 完整；
  - 无 Content-Length 的 StreamingResponse → 不 gzip，内容完整；
  - 小 `text/csv` → gzip；`application/zip`/video/jpeg → 不 gzip（现有用例保持）；
  - Range 请求 → 不 gzip（现有）。
- 文档：部署FAQ 补「为什么大导出没有 gzip（保护内存）」；35-api-reference 不必改。

## 约束

- 不改变小响应压缩行为/缓存头；不引入依赖；
- 不破坏 CSV 流式导出（前端下载内容与未压缩一致）；
- 测试用 TestClient/临时数据，不生成真实 GB 级文件。

## 验收

- `pytest tests/ -q` 全绿（基线 165 + 新增）；
- 8001 实跑：小 JS gzip、`/api/export/events.csv`（造 >4MB 或 mock）不 gzip 且下载完整；
- 报告附响应头证据。

## 执行报告

（待填：改动文件 / 阈值实测 / pytest / 遗留）
