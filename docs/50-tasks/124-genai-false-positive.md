# 任务 124 · 本地 VLM 误报预分类（P2，直击模型止损线）

> 来源：flash 建议 #12（Frigate `genai` 蓝本 + Immich 推理微服务形态）。
> 目标不是替代 YOLO，而是在事件落库后对截图做**二次判读**，标记“疑似误报/疑似真实/不确定”，
> 供人工优先处置与误报率统计；**绝不自动修改事件 status**。默认关闭，失败静默降级。

## 目标

1. 新增可选 GenAI 复核（`server/app/genai.py`）：
   - provider：`ollama`（`POST /api/generate`，图片 base64）与 `openai_compat`
     （`/v1/chat/completions`，OpenAI 兼容本地服务）；默认 `none`。
   - 配置（`LS_`）：`GENAI_PROVIDER=none|ollama|openai_compat`、`GENAI_URL`（默认
     `http://127.0.0.1:11434`）、`GENAI_MODEL`、`GENAI_API_KEY`（可选）、`GENAI_TIMEOUT_S=30`、
     `GENAI_MAX_CONCURRENCY=1`、`GENAI_MIN_LEVEL=2`（只复核 >= 级别）。
   - **默认关闭**；地址默认回环；非回环需显式 `GENAI_ALLOW_REMOTE=true`（防误配外发截图）。
2. 事件落库后异步复核（独立单线程队列，不阻塞 pipeline/告警）：
   - 取事件截图（无截图跳过）；压缩到长边 ≤1280 JPEG 后发送；
   - 输出严格 JSON：`{verdict: "likely_true"|"likely_false"|"uncertain", reason: string<=200,
     confidence: 0..1}`；解析失败/超时/HTTP 错误 → 不写结果、只记 WARNING/指标。
   - 写入 `events.ai_review`（TEXT JSON，迁移版本用当时最新+1）并保留 `ai_review_ts/model`；
     API 事件行返回 `ai_review` 对象与 `ai_verdict` 字段；前端事件详情显示徽标
     “AI：疑似误报（仅供参考）”，并提供筛选（可选）。
3. 统计：`/api/stats/summary` 或 `/api/events/quality` 增加 `ai_verdict` 计数（供 102 使用）。
4. 绝不自动改 `status`/不抑制告警；VLM 失败不影响事件、投递、WS。

## 提示词与输出

- prompt 固定为“实验室安全监控截图复核”，要求只输出 JSON、字段长度受限；
  示例：
  ```
  你是实验室安全监控复核员。看这张事件截图，判断：画面中是否确有人员未穿实验服/进入禁区？
  只输出 JSON：{"verdict":"likely_true|likely_false|uncertain","reason":"<=200字","confidence":0-1}
  无法判断时 verdict=uncertain，不要臆测。
  ```
- 解析前剥离 markdown 代码围栏；JSON 必须通过 pydantic 校验（`extra="forbid"`）。

## 实现建议

- `genai.py`：provider 注册表 + `review_event(event_row)`；队列 `queue.Queue(maxsize=100)`，
  worker 线程 daemon；满队列丢弃并计数（不阻塞）。
- 与 DML 的关系：VLM 走 Ollama/独立服务进程，不复用 `Engine._run()`；但同机 GPU 可能竞争，
  默认并发 1 + 超时；文档要求 24h/压测时关闭。
- 配置校验（validate.py）：provider 非 none 时 URL/model 必填，失败 WARN/FAIL；token 脱敏。
- 诊断包脱敏：`GENAI_API_KEY` 加入 61 脱敏清单。

## 测试（`server/tests/test_genai.py`）

- 假 HTTP transport：ollama/openai_compat 请求体/图片字段正确，JSON 解析成功；
- 坏 JSON/超时/500 → 不写 ai_review、事件仍存在、告警不受影响；
- 默认 none → 零网络请求、零结果；
- 非回环 URL 未 allow → 拒绝；
- 队列满丢弃不阻塞；worker 单并发（fake 并发计数 ≤1）；
- 事件 API 返回 ai_review；DB 迁移旧库兼容。
- 真实 Ollama 验证：若本机有可用模型，抽 5 张事件截图人工看结果；没有则记
  `⏳ BLOCKED: 本机无 Ollama/模型`，不下载大模型。

## 验收

- `pytest tests/ -q` 全绿；默认配置下零行为变化、零外部请求；
- 报告附：provider/模型、5 条真实或样例截图的 verdict 与人工对照（没有就写 BLOCKED）；
- 明确“AI 只做预分类，不改处置状态；误报率仍以人工处置为准”。

## 约束

- 不新增 Python 依赖（httpx 已有）；不下载模型到仓库；不把截图发到非回环地址（除非显式 allow）；
- 不自动抑制告警、不改 status；GPU 竞争风险写进文档。
