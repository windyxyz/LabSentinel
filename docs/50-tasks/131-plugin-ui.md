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
