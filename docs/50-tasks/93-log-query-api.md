# 任务 93 · 日志查询 API + 前端日志面板（P2）

> 背景：任务 61 有了轮转日志和诊断包，但排障时还要远程让用户找文件。加一个只读日志尾部查询，
> 管理员/值班可直接在页面看最近错误（脱敏后）。

## 目标

### 1. `GET /api/logs`（读保护口径）

参数：
- `tail=200`（1–1000，默认 200）
- `level=ERROR|WARNING|INFO`（可选，大小写不敏感；按行解析级别）
- `q=`（可选，子串，长度 1–100；转义无特殊语义，纯子串）
- `file=server.log`（固定白名单：server.log / server.log.1 / server.log.2 / server.log.3）

返回：
```json
{"file":"server.log","lines":["..."],"matched":12,"scanned":200,"truncated":false}
```
- 只从文件**尾部**读最多 2MB（避免大文件内存），按行倒序取匹配后正序返回最近 tail 条；
- 行内 URL 凭证 `user:pass@` 脱敏为 `***:***@`；不返回其他文件/路径；
- 文件不存在 → 200 `{lines:[],matched:0,note:"日志文件不存在（可能 LS_LOG_FILE=0）"}`；
- 不支持翻页历史（只尾部），文档写明。

### 2. 前端 `web/src/views/Reports.vue`（或 Cameras 底部）

- 「服务日志」折叠区：文件选择（server.log/轮转）、级别筛选、关键词、刷新；
- 等宽 `<pre>` 展示最近 200 行（超长横向滚动，不撑破 375px）；
- 错误行红色高亮（简单按 ` ERROR ` 判断）；空态/失败提示；
- 只改一个 view（优先 Reports），不新增路由。

### 3. 测试 `server/tests/test_log_query.py`

- 临时日志文件 300 行（含 INFO/WARNING/ERROR、URL 凭证）→ tail=50 返回最近 50、level=ERROR 只匹配 ERROR、
  q 子串过滤、凭证已脱敏；
- 文件缺失/轮转文件不存在 → 200 空 + note；
- tail 边界 0/1001 → 400 或夹取（定义一种）；
- 鉴权：读保护开时无 token 401。

### 4. 文档

- `docs/35-api-reference.md` 端点；
- `docs/运维手册.md` 日志诊断一节加页面查看；
- `docs/部署FAQ.md`「不想下载诊断包，怎么快速看错误」。

## 约束

- 只读、固定文件白名单、防路径穿越；不引入依赖；
- 脱敏 URL 凭证；不返回完整绝对路径（只文件名）；
- 前端只改一个 view。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 实跑：制造一条 ERROR（如请求不存在资源）→ 页面/端点可见；凭证脱敏；
- 报告附响应样例（脱敏）。

## 执行报告

（待填：改动文件 / 响应样例 / 测试与 build / 遗留）
