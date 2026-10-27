# 任务 31 · API 快速上手与 curl 示例（先读本目录 README.md）

## 目标（纯文档）

在 docs/35-api-reference.md 末尾追加（或在 docs/ 新建 `API快速上手.md` 并互链，二选一，执行报告说明）：

1. **5 分钟上手**：启动服务 → 添加文件摄像头（autostart=false）→ 建安全区 → 加规则 → 启动 → 查事件 → 清理。每步给 **curl（Git Bash 版）** 与 **PowerShell 版**（`Invoke-RestMethod`）两种写法
2. **curl 注意事项**（来自真实踩坑）：
   - Git Bash 发含中文 JSON 会被转码损坏 → 推荐 `--data-binary @file.json`（UTF-8 文件）或使用 python httpx；PowerShell 用 `-Body ($json | ConvertTo-Json)` 并注意 `chcp 65001`
   - 鉴权：`LS_ADMIN_TOKEN` 启用后写操作带 `-H "X-Admin-Token: ..."`；WS 连接 `?token=`
   - WebRTC 端点返回的 ws_url 是给浏览器用的（go2rtc 1984）
3. **端到端示例脚本引用**：指向 `tools/e2e_smoke.py`（黑盒全链路）与 `tools/loadtest.py`
4. **错误码速查**：400/401/403/404/502 各自常见原因（对照实现）

## 约束

- 每个示例都必须在当前实现上可运行（写前对照 api.py 校验参数名）；不能编造端点
- 中文请求体一律用"写进 UTF-8 文件再 @file"的方式演示

## 验收标准

上手段落按文档逐条执行可跑通（服务运行时抽 2 条实测）；无虚构端点；与 35-api-reference 一致。

## 执行报告

### 交付物
- `docs/35-api-reference.md` 末尾新增「10. 5 分钟上手」章节（**追加方案**，保持 API 单文档，报告注明）：
  - 10.1 Git Bash curl 版八步（摄像头→安全区→规则→启动→事件→截图视频→处置→清理）
  - 10.2 PowerShell Invoke-RestMethod 版（UTF-8 文件 -InFile 方式处理中文 body）
  - 10.3 真实踩坑表：Git Bash 中文 JSON 损坏（@file/httpx）、PowerShell chcp 65001、
    鉴权 X-Admin-Token 与 WS ?token=、webrtc ws_url 是浏览器信令非 curl 可建连、视频需 OpenH264
  - 10.4 端到端脚本指引（tools/e2e_smoke.py / tools/loadtest.py）
  - 10.5 错误码速查 400/401/403/404/502/503（对照 api.py 实现）

### 实测记录（验收要求抽 2 条，对运行中的 8000 实例）
- `GET /api/system` ✓（返回 backend=directml 与 4 路摄像头状态）
- `GET /api/events?limit=1` ✓（返回完整事件行，含 status/handled_by 新字段）
- `POST /api/events/1/status`（空 body）→ 400 ✓（端点存在且校验生效）
- **发现①**：`GET /api/events/count?camera_id=1` → 404 —— 运行中的预览实例启动于任务 19 之前
  （旧代码），**需重启服务端加载 12-26 新功能**；文档已加版本提示
- **发现②（记给主线）**：events 列表响应含 snapshot/video 的服务器本地绝对路径，建议后续改为只回
  has_snapshot/has_video 布尔（改前不动契约）

### 验收结果
- 端点/参数名逐一对照 api.py 实现（零虚构）；示例两条实测通过；与 35-api-reference 正文一致
- 纯文档，不改代码
