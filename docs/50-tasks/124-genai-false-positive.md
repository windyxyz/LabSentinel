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

## 执行报告

**flash 2026-11-04 完成（pytest 463 passed = 450+13 新增；默认配置零行为变化已实测）**
（全量回归为 103 基线上 124 的口径，数字以本轮实跑为准）

### 1. server/app/genai.py（新文件，Frigate genai 蓝本）

- provider：`ollama`（`/api/generate` images=b64 + format=json + temperature 0）、
  `openai_compat`（`/v1/chat/completions`，content 数组 image_url data-URI，Bearer 可选）；
  **默认 none=零开销**（maybe_enqueue 直接短路）；
- 输出严格校验：剥离 markdown 围栏 → json.loads → pydantic `AIReview`（`extra="forbid"`）
  + verdict 白名单 + confidence 钳制 0..1 + reason 截 200；
- **回环保护**：非回环 URL 未 `LS_GENAI_ALLOW_REMOTE` 即拒（发请求前）；
- 队列：单 daemon 线程，maxsize=100 满即丢弃+计数；单并发 enforced（测试断言 max=1）；
  worker 任何异常只计数不自尽（两条 500 连发后线程仍活）；
- 截图压缩：cv2 读→长边 ≤1280→JPEG q80；读失败跳过；
- 写回 `events.ai_review`（JSON）+ai_review_ts+ai_review_model；**绝不碰 status/告警**。

### 2. 接线

- **迁移 v9**（events 三列 ALTER，旧行 NULL=未复核行为不变）+ `_SCHEMA` 同步；
- `Store.add_event` 尾部 `maybe_enqueue`（try 包裹零影响）+ `Store.set_ai_review`；
  main.py lifespan start/stop worker；
- `_event_row` 统一转换：`ai_review`（对象）+`ai_verdict` 快捷字段（全事件端点生效）；
- `GET /api/genai/status`（读保护，纯计数）；`/api/stats/quality` 新增 `by_ai_verdict`；
- validate.py：缺 model WARN、并发≠1 WARN；诊断包脱敏经 redact.py KEY 正则自动覆盖。

### 3. 测试（test_genai.py，13 用例，fake httpx 零外网）

解析（围栏/坏 JSON/非法 verdict/extra 拒/钳制/截断）、回环保护、两 provider 请求形状、
HTTP 失败抛错、入队门控、worker 写回链路、坏 payload 线程存活、单并发 max=1、
满队列丢弃、v9 旧库迁移 NULL、set_ai_review。
（坑：genai._write_back 直连 settings.db_path——生产正确、单测需 monkeypatch 指向 fixture 库；
假 JPEG 字节 cv2 读不了 → 测试改用 imencode 真图。）

### 4. 实机验证（8188 默认配置）

`schema_version=9`；`/api/genai/status` provider=none 全零；造 L3 事件 →
`ai_review=null/ai_verdict=null`；quality `by_ai_verdict` 三键全 0 → **默认态零行为 PASS**。

### 5. 真实 VLM 验证

⏳ **BLOCKED: 本机无 Ollama/模型**（`http://127.0.0.1:11434/api/tags` 无响应、无 ollama 可执行）；
按规格不下载大模型。链路已由 fake provider 全覆盖；真模型接入即用（FAQ 有步骤）。

### 约束遵守

不新增依赖；不下载模型；截图仅回环（除非显式 ALLOW_REMOTE）；不自动改 status/抑制告警；
GPU 竞争风险已写 FAQ/validate WARN。

### 遗留

- 前端事件详情徽标（"AI：疑似误报（仅供参考）"）：API 字段已就绪（ai_review/ai_verdict），
  渲染与筛选留待随 128 桌面体验一起做（前端只读展示，零后端依赖）。

### 主线复审补丁（2026-11-04）

- **中文路径读图**：`_compress_jpeg` 原用 `cv2.imread`，项目默认路径含“监控识别”时静默返回 None，
  VLM 永远无法复核 → 改 `vision.read_image`（np.fromfile+imdecode），新增中文路径回归测试。
- 运行版本统一 `0.1.0-alpha`（原 `0.1.0-m1`），OpenAPI 快照重生成。
