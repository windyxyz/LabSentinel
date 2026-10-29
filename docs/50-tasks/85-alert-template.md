# 任务 85 · 告警文案模板可配置（P2）

> 背景：`render_alert` 的文案写死在代码里。不同单位希望标题/正文带自己名称、值班电话，或去掉某些
> 字段。本任务支持用配置模板渲染，并提供预览接口，避免改代码。

## 目标

### 1. 模板配置

- 配置项（`LS_` 前缀）：
  - `alert_template_title: str = "LabSentinel 级别{level}告警"`
  - `alert_template_text: str` 默认等价于现有 `render_alert` 输出：
    `"{icon} L{level} {type_text} · {ts}\n摄像头 #{camera_id} · track {track_id}\n{detail}"`
    （occupancy_over 默认不含 track；模板渲染时 track_id 为空串处理）
- 占位符白名单：`icon/level/type/type_text/ts/camera_id/track_id/detail`；
  - 未知占位符 → 原样保留（不抛异常），并记录一次 WARNING；
  - 缺失字段 → 空串；`detail` 中的花括号不做二次格式化（用 `format_map` + 单次替换）。
- `render_alert(row)` 改为走模板；`alert_title(level)` 同理。
- 兼容：不配置时输出与当前**逐字一致**（用现有测试快照/断言保证）。

### 2. 预览/测试

- `GET /api/alert-template`（读保护口径）：返回当前 title/text 模板 + 每个占位符说明 + 默认值；
- `POST /api/alert-template/preview`（require_admin）：body `{level, type, camera_id, detail, track_id}`
  → 返回渲染后的 title/text（不发送、不写库）；
- 现有 `/api/alert-test` 使用当前模板发送测试消息。

### 3. 前端（`web/src/views/Cameras.vue` 告警面板）

- 「告警文案」区块：显示当前模板（只读 textarea）、占位符说明、「预览」按钮（可编辑示例字段，
  调 preview 展示结果）；模板修改通过 `.env`/`config.env`，界面注明“改配置后重启生效”；
- 不做在线保存（避免配置写入权限/并发问题），报告说明这是刻意取舍。

### 4. 测试 `server/tests/test_alert_template.py`

- 默认模板输出与旧 `render_alert` 断言逐字一致；
- 自定义 title/text（含未知占位符/中文/换行）→ 渲染正确、未知占位符原样保留；
- `format_map` 注入测试：detail 含 `{level}` 不被二次替换；
- preview 端点 200/鉴权/字段缺失；alert-test 使用模板（mock 渠道断言收到文本）。

### 5. 文档

- `.env.example` 两项注释 + 占位符表；
- `docs/部署FAQ.md`「怎么改告警文案 / 支持哪些占位符」；
- `docs/运维手册.md` 告警配置一节。

## 约束

- 不改变现有渠道发送/重试逻辑；不引入模板引擎依赖；
- 默认输出逐字兼容（现有 `test_push`/`test_alerts_page` 不得改断言来迁就）；
- 不在 UI 写 `config.env`；不记录模板渲染中的敏感值。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 实跑 preview（自定义模板）+ alert-test 到本地 stub；
- 报告附旧/新渲染对照，证明默认不变。

## 执行报告

（待填：改动文件 / 默认兼容证据 / 测试与 build / 遗留）
