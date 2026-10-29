# 任务 87 · 操作审计日志（P1）

> 背景：现在谁删了摄像头、谁改了规则、谁批量误报，都没有记录；出问题无法追溯。本任务对**所有
> 非 GET 的 `/api/*` 写操作**自动落审计（不记录请求体，避免密码/令牌入日志），并提供管理员查询。

## 目标

### 1. 表（迁移 v4）

```sql
CREATE TABLE audit_log(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,
  request_id TEXT,
  actor TEXT,              -- admin | read | anonymous
  action TEXT NOT NULL,    -- "POST /api/cameras"（方法+路径，不含 query）
  target TEXT,             -- 路径参数摘要（如 camera:3 / event:12），可空
  status_code INTEGER,
  detail TEXT              -- 可选短说明（不写 body）
);
CREATE INDEX idx_audit_ts ON audit_log(ts);
```

### 2. 自动记录（中间件）

- 在 `main.py` 加 HTTP 中间件（放在请求 ID 之后，确保能拿到 request_id）：
  - 仅 `method in POST/PUT/PATCH/DELETE` 且 path 以 `/api/` 开头；
  - 请求完成后记录：ts、request_id、actor（由令牌头判断：admin/read/anonymous）、action、status_code；
  - **不记录** body/headers/query（防止密码/令牌）；`target` 可从 path 末段数字提取（`/api/events/12/status`→`event:12`）；
  - 写库失败只 WARNING，不影响响应；
  - 为减少噪声，`/api/health/*` 不记（本来非写）；`/api/alert-test`、`/api/maintenance/cleanup` 记。
- 批量端点（83）一条审计即可（action 为路径），不逐 id 记录。

### 3. 查询 API/UI

- `GET /api/audit?limit=50&offset=0&action=&actor=&start_ts=&end_ts=`（require_admin；limit 1–200）：
  返回 `{items:[...], total}`；`total` 用同条件 count。
- `web/src/views/Reports.vue` 增加「操作审计」折叠区：最近 50 条表格（时间/操作者/动作/状态码/request_id），
  支持 actor/动作前缀筛选与翻页；空态“暂无写操作”。
- 不新增路由/不改进航。

### 4. 保留

- `retention.run_cleanup` 同时删除 `audit_log` 中超过 `retention_events_days` 的行（与事件同口径），
  统计加入 `deleted_audit`；预演也统计。

### 5. 测试 `server/tests/test_audit.py`

- TestClient 调 `POST /api/cameras`（临时库）→ audit 行存在、action/status/request_id/actor 正确；
- GET 不产生审计；401 的写请求也记录（status=401, actor=anonymous）；
- 查询 API：分页/筛选/total；无 admin token 401；
- body 中的敏感串不出现在 audit 任何列；
- retention 清理过期 audit 行；预演只读。

### 6. 文档

- `docs/35-api-reference.md` 新增查询端点；
- `docs/部署FAQ.md`「怎么查谁删了摄像头」；
- `docs/运维手册.md` 安全合规/审计一节。

## 约束

- 绝不记录请求体/headers/query；不引入依赖；
- 中间件异常不能影响业务响应；测试临时库；
- 审计表不参与备份之外的特殊处理（backup 已整库备份）。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 实跑：添加/删除摄像头后查询审计可见，request_id 与响应头一致；
- 报告附审计行 JSON（脱敏）。

## 执行报告

（待填：改动文件 / 审计证据 / 测试与 build / 遗留）
