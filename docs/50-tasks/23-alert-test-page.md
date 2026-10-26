

## 执行报告

### 交付物
- **23A 服务端**：`GET /api/alerts/status`（只读）——七渠道遍历（wecom/dingtalk/feishu/serverchan/
  pushplus/qmsg/email），`configured` 按各自 settings 字段组合判定（qmsg 需 key+qq、email 需 host+to，
  **响应不含任何敏感值**）；`deliveries` 从 alert_deliveries 按 channel×status 汇总
  （`store.delivery_counts_by_channel()` 新方法）。未改任何发送逻辑
- **23B Web**（Cameras.vue 内嵌"告警设置"区块，**未新建顶级导航**，未动 App.vue）：
  - 渠道状态表：中文名 / 已配置✓·未配置（令牌色徽章）/ ok·failed·pending 计数（failed 红色高亮）
  - 「发送测试告警」按钮 → POST /api/alert-test → 汇总结果与逐渠道失败原因
  - 失败投递列表（GET /api/alert-deliveries?status=failed 最近 10 条）+ 单条「重发」（retry 端点）
  - 配置说明指向部署指南「4. 告警渠道配置」；样式沿用主题类，无全局改动

### 验收结果
- `pytest tests/ -q`：**74 passed**（新增 3：渠道遍历+configured 组合判定 / 敏感值零泄露 /
  deliveries 按渠道汇总计数）
- `npm run build` 通过（gzip 42.4KB）

### 遗留问题
- 渠道"已配置"仅代表 settings 非空，不代表可达——可达性用「发送测试告警」验证（设计如此）
- CHANNEL_NAMES 中文映射在 Cameras.vue 内（与 constants.py 的 TYPE_TEXT 不同源，属 UI 文案；多端复用时再抽）
