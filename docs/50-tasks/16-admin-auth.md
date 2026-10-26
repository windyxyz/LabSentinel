# 任务 16 · 管理鉴权（先读本目录 README.md）

## 背景

服务端无鉴权（局域网内网部署）。需要一层轻量保护：防止同网段其他人误操作/恶意删配置。不做多用户体系（M3 再议）。

## 目标（服务端为主）

### 16A 简单 Token 鉴权
- 配置：`LS_ADMIN_TOKEN`（.env.example 同步注释）。**为空 = 不启用鉴权**（兼容现状/内网信任场景）
- 启用后规则：
  - 所有 **写操作**（POST/PUT/DELETE）必须带 `X-Admin-Token: <token>`，否则 401
  - 读操作默认放行；`LS_AUTH_READ_PROTECTED=true` 时可要求同样 header（可选）
  - `/api/system` 永不鉴权（健康检查）
- 实现：FastAPI 依赖注入（`Depends(require_admin)`）挂在写路由上；比较用 `hmac.compare_digest`
- WS `/ws/events`：连接 query `?token=`，启用鉴权且不匹配则 close(4401)

### 16B Web 端
- `api.js`：从 localStorage 读 token 附带 header；设置页（Settings 概念不存在——放"摄像头"页顶部或导航底部）提供 token 输入 + 保存
- 401 时全局提示"需要管理令牌"，跳 token 输入

### 16C 文档
- 部署指南补一节；API 文档标注哪些端点受保护

## 约束

- 不引入 JWT/会话库；无状态 header 即可
- token 不入日志、不入 URL（WS 除外，注明风险）
- 默认空 token（不启用）→ 现有测试零回归

## 验收标准

pytest 全绿 + 新增 ≥4 测试（无 token 401 / 错误 token 401 / 正确放行 / 空 token 不启用）；API 文档同步。

## 执行报告

### 交付物
- **16A Token 鉴权**：`config.py` 新增 `LS_ADMIN_TOKEN`（**空 = 不启用**，零回归）与
  `LS_AUTH_READ_PROTECTED`（读保护开关，默认关）；`api.py` 增加 `_token_ok`（**hmac.compare_digest**）
  与两个守卫 `require_admin` / `require_read`，以 `Depends` 挂载：
  - ⚡写保护（14 个端点）：摄像头增删/启动、安全区增删、规则增删、事件处置、人脸注册/删除/一键清除、
    死信重发、渠道自检、维护清理
  - 读保护（5 个端点，仅开关打开时生效）：events 列表/截图/视频、face persons、alert-deliveries
  - `/api/system` 永不拦截；`WS /ws/events?token=` 失败 close(4401)
- **16B Web**：`api.js` 导出 `adminToken`（localStorage 持久化），req 自动附带 `X-Admin-Token`，
  401 时错误信息明确提示"到摄像头页填写令牌"；Cameras.vue 顶部新增"管理令牌"设置区（保存/清除）
- **16C 文档**：部署指南新增 3.5 节（启用方式/受保护范围/WS 风险注记/验收方法）；
  35-api-reference.md 通用约定标注 ⚡写保护；.env.example 同步两条配置

### 验收结果
- `pytest tests/ -q`：**63 passed**（新增 4：token 关闭零回归 / 启用后 无-错-对 三态 + system 永不拦 /
  读保护开关 / WS 4401 与带 token 连接）
- `npm run build` 通过（gzip 38.4KB）；token 不入日志（守卫只做比较，无打印）；WS token 在 URL（文档已注明风险）
- 未引入任何 JWT/会话依赖

### 遗留问题
- 处置人 handled_by 仍是请求方可传字段（鉴权与身份无关）；M3 多用户再接
- cameraWebrtc 的 ws_url 未拼 token（go2rtc 侧有自身 origin 配置；如需端到端可后续在 webrtc 端点校验 admin 后拼 token）
