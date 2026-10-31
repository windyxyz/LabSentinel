# 任务 88 · 只读令牌（P1）

> 背景：现在只有一个 `LS_ADMIN_TOKEN`：给了就是全权限，不给就全开放。值班/大屏只需要看，
> 不应该持有可删除摄像头/改规则的令牌。本任务增加只读令牌，写操作仍只认管理令牌。

## 目标

### 1. 配置与鉴权语义

- 新增 `read_token: str = ""`（`LS_READ_TOKEN`）。
- 规则（明确写进文档）：
  - `admin_token` 为空 → 鉴权整体不启用（保持现状；`read_token` 被忽略并启动 WARNING）；
  - `admin_token` 非空：
    - 写操作（现有 `require_admin`）只接受 **admin_token**；
    - 读操作（`require_read` + 默认公开的 GET）：
      - `auth_read_protected=false` 时保持公开（与现状一致，只读令牌不改变公开读）；
      - `auth_read_protected=true` 时接受 admin_token 或 read_token；
  - 若 `auth_read_protected=false` 但设置了 read_token，GET 仍公开（在文档说明：要保护读必须同时开
    `LS_AUTH_READ_PROTECTED=true`）。
- `read_token` 与 `admin_token` 相同 → 启动 WARNING（弱隔离），但允许运行。

### 2. WS 鉴权

- `/ws/events` 的令牌校验接受 admin 或 read（读事件流属只读）；写事件仍只认 admin。
- 现有 WS 4401 行为保持；测试覆盖 read token 可连、错误 token 拒绝。

### 3. 前端

- Cameras 页令牌设置区文案改为「管理令牌（写操作）/ 只读令牌（只读查看）」说明；
- `api.js` 仍发送同一个 `X-Admin-Token` 头（用户填哪个就用哪个）；写操作 401 文案提示
  “当前令牌无写权限，请换管理令牌”；
- 不做双令牌存储（避免复杂）；报告说明取舍。

### 4. 测试 `server/tests/test_readonly_token.py`

矩阵：
- admin 空、read 任意：全部开放（写/读/WS）。
- admin 非空、read 空：写 admin✓/其它✗；读保护开时 admin✓、匿名✗。
- admin+read：读保护开时 read 可读、read 写 401；admin 全通；WS read 可连、错误令牌 4401。
- read==admin 启动 WARNING（caplog）。
- 现有 `test_auth.py` 不回归。

### 5. 文档

- `docs/部署指南.md` §3.5 鉴权：两令牌权限矩阵；
- `docs/部署FAQ.md`「大屏/值班只读怎么配」；
- `.env.example` 加 `LS_READ_TOKEN` 注释。

## 约束

- 不改变 admin_token 为空时的现有开放语义；不引入依赖；
- 不实现多用户/角色表（超出范围，报告注明）；
- 令牌比较仍用 `hmac.compare_digest`。

## 验收

- `pytest tests/ -q` 全绿（含现有 auth 测试）；
- 8001 实跑：admin+read、读保护开，read GET 200、read DELETE 401、admin DELETE 200；
- 报告附权限矩阵实测结果。

## 执行报告

**flash 2026-10-31 完成（pytest 248 passed、build 50.49KB、权限矩阵 7 用例全过）**

- **改动**：
  - `config.py` 加 `read_token: str = ""`（LS_READ_TOKEN）；
  - `api.py`：新增 `_read_token_ok()`（admin_token 空→恒 True；否则 admin 或 read 令牌均可，
    hmac.compare_digest）；`require_read` 改用 `_read_token_ok`（admin/read 均可读）；
    `require_admin` 保持只认 admin_token——**只读令牌写操作 → 401 "当前令牌无写权限"**（可区分
    于普通 401）；WS `ws_events` 改 `_read_token_ok`（读事件流属只读，read 令牌可连）；
    `_warn_same_token()`（read==admin 时启动 WARNING 弱隔离提示）。
- **权限矩阵（测试实测）**：

| | 匿名读 | read 读 | admin 读 | read 写 | admin 写 |
|---|---|---|---|---|---|
| admin=""（不启用） | 200 | 200 | 200 | 200 | 200 |
| admin 设 + read="" + 读保护 | 401 | 401 | 200 | 401 | 200 |
| admin+read 均设 + 读保护 | 401 | **200** | 200 | **401** | 200 |
| admin+read 均设 + 读保护关 | 200 | 200 | 200 | 401 | 200 |

- **测试**（test_readonly_token.py 7 用例）：上述矩阵全覆盖 + WS read 可连/错误令牌拒绝 +
  read==admin WARNING（capsys）+ **auth_read_protected=false 时设了 read_token GET 仍公开**
  （文档口径）。既有 test_auth.py 全部不回归。全量 **pytest 248 passed**（241 + 7）；
  build gzip **50.49KB**。
- **实施中修复的 2 个 bug**：①`require_read` 的 `if not provided or not _read_token_ok(provided)`
  在 provided=None 时短路——admin_token="" 时匿名读被误 401（改为先调 _read_token_ok）；
  ②`_read_token_ok` 在 admin_token="" 时须恒 True（与"鉴权整体不启用"语义一致）。
- **文档**：部署指南 §3.5 权限矩阵；部署FAQ「大屏/值班只读怎么配」；.env.example 加 LS_READ_TOKEN。
- **遗留**：无。前端 api.js 仍发同一个 X-Admin-Token 头（用户填哪个就用哪个）——不做双令牌
  存储（刻意取舍，避免复杂）；写 401 detail 已含"无写权限"提示。
