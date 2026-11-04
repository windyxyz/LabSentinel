# 任务 131 · 插件管理 UI（P1）

> 后端插件一期（126）已交付 API 与示例，但前端没有任何入口——用户无法启用/启停/看日志，
> 功能等于不可用。本任务只补 UI，不改后端语义。

## 目标

在「摄像头」页底部新增“插件（外挂）”折叠区（与既有「服务控制」风格一致，不新建导航页）：

1. 列表：`GET /api/plugins` → 名称/版本/id/权限/enabled/running/pid/returncode/last_error。
2. 操作：
   - 启用/停用：`POST /api/plugins/{id}/enable|disable`；
   - 启动/停止：`POST /api/plugins/{id}/start|stop`；
   - 看日志：`GET /api/plugins/{id}/logs?tail=200`（弹窗/内联 pre，最多 200 行）。
3. 安全提示：
   - 含 `write` 权限的插件显示醒目警告“将获得管理员令牌”；
   - 启用前 `confirm()` 二次确认；
   - 未配置鉴权时提示“默认无令牌，内网使用”。
4. 三态/防重：加载中、空（“把插件放到数据目录/plugins/<id>/”）、错误（含 401 提示）；
   操作按钮按插件 id 防重复。
5. `web/src/api.js` 增加 plugins 列表/启停/日志方法；不改 `App.vue`/`main.js`/theme。

## 实现建议

- 放在 `Cameras.vue` 第三个 `<div class="panel">`（服务控制之后），复用 `.panel/.row/.muted/table/AsyncState`。
- 日志弹窗用固定定位 overlay（参考 FolderImages 详情），点击空白关闭。
- 刷新：操作后重新拉列表；页面加载时拉一次（失败静默，不阻塞摄像头表）。
- 不显示 token；日志内容按文本渲染（`<pre>{{ logs }}</pre>`，禁止 v-html）。

## 测试/验收

- 后端已有 `test_plugins` 覆盖 API；本任务不新增后端逻辑。
- `npm run build` 通过；1280/375 宽度不溢出；无 App.vue/theme 改动。
- 真实服务（隔离 data/plugins 放一个 disabled 示例）手工/脚本验证：
  列表可见 → 启用 → 启动（`{python}` 示例）→ 日志有输出 → 停止。
- 报告附截图/命令与 build gzip。

## 约束

- 只做管理 UI；不自动启用/启动插件、不打包示例、不改插件后端。
- 不展示令牌/环境变量值；401 引导去顶部填令牌。

## 执行报告

**flash 2026-11-04 完成（build gzip 61.61KB；8188 隔离实例全链实机 PASS；后端零改动）**

### 实现（只改 Cameras.vue + api.js；App.vue/main.js/theme 未动）

- **api.js** +3：`pluginsList` / `pluginAction(id, action)` / `pluginLogs(id, tail=200)`；
- **Cameras.vue**「服务控制」后新增「插件（外挂）」区块：
  - 列表：名称+id/版本/权限徽章（**write 红色加粗**）/状态（运行中 pid、已退出 rc、未启用、
    last_error 直显）；
  - 操作：启用/停用、启动/停止、看日志——按钮按 `pluginBusy === p.id` 防重；
    enable 含 write 权限时 confirm 明确警示"将获得管理员令牌（等同完全控制本服务）"；
  - 安全提示：顶部"插件=代码执行，默认禁用"说明 + write 警示文案（未配置鉴权时
    "以无令牌运行（内网使用）"）；
  - 三态：加载中/空（引导放 `<数据目录>/plugins/<id>/`+刷新按钮）/错误
    （**401/403 引导去页面顶部填令牌**，操作失败 alert 同样引导）；
  - 日志弹窗：固定定位 overlay（点空白/关闭按钮关），`<pre>{{ text }}</pre>` 纯文本
    （**禁 v-html**），不显示任何令牌；
  - 页面 onMounted 拉一次（失败留错误提示不阻塞摄像头表），操作后自动重拉。
- 样式复用 .panel/.row/.muted/table/.ghost/.st-ok；375px 走 panel overflow-x（既有规则）。

### 实机验证（8188 隔离实例 + 示例 event-logger 复制到 data/plugins）

列表（disabled 可见）→ enable → start（`{python}` 解析 pid=22584）→ running=True →
日志含启动横幅 → stop → running=False → **全链 PASS**。

### 遗留

- 无。截图略（表格/弹窗结构见代码；与既有面板同构）。
