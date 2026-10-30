# 任务 115 · PWA 安装 + 浏览器通知（P2，Windows 优先）

> 背景：Windows 桌面用户希望“像应用一样”打开面板，并在页面最小化时收到新事件提醒；现在只有网页。
> 本任务加 PWA manifest/service worker（离线 app shell）与浏览器通知，**不做手机 APK**。

## 目标

### 1. PWA

- `web/public/manifest.webmanifest`：name/short_name/start_url `/`/display `standalone`/
  theme_color/background_color/icons（192/512 PNG；可用脚本生成简洁盾牌图标，放 `web/public/icons/`）。
- `web/public/sw.js`：
  - 安装时缓存 app shell（`/`、`/index.html`、构建后的 assets 由 fetch 缓存）；
  - fetch：导航请求 network-first（离线回退缓存 index）；静态 assets cache-first；`/api/*`、`/ws/*`
    **不缓存**（直接网络，失败返回错误）；
  - activate 清理旧版本缓存；`skipWaiting` + `clientsClaim`；
  - 版本常量与构建产物 hash 无关，更新时 bump。
- `web/index.html`：加 manifest link、theme-color、apple-touch-icon；用**内联 module script** 注册
  `/sw.js`（不改 `main.js`）；仅 `location.protocol` 为 http/https 时注册。
- 注意：service worker 需要 localhost 或 HTTPS；局域网 HTTP 下不生效，文档说明。

### 2. 浏览器通知

- `web/src/notify.js`（新）：
  - `ensurePermission()`：请求 `Notification.permission`；
  - `notifyEvent(e)`：页面 hidden 且有权限时 `new Notification(标题,{body,tag})`；
    tag=`cam-<id>-<type>` 防重复；点击 `window.focus()`；
  - 同一 tag 10s 内不重复；L1 不通知（只 L2/L3）；可配置开关（localStorage）。
- `Dashboard.vue`：加“开启桌面通知”按钮（权限状态显示）；在现有 WS `onEvent` 中调用 `notifyEvent`；
  不新增路由/不改全局四件套。

### 3. 测试/验收

- `npm run build` 通过；`dist/manifest.webmanifest`、`dist/sw.js`、icons 存在；
- 用 Playwright（或手工）验证：manifest link 可访问、`navigator.serviceWorker.register` 成功
  （localhost）；断网后刷新返回 app shell（至少 index）；
- 通知权限拒绝时不报错；hidden=false 不弹；
- 不缓存 `/api` 的断言（检查 sw.js 逻辑/用 mock fetch 单测可选）。

### 4. 文档

- `部署FAQ`「怎么把面板装成应用/收桌面通知（需 localhost 或 HTTPS）」；
- `运维手册` 值班通知一节；README 功能列表加一行（可选）。

## 约束

- **禁止修改** `web/src/main.js`（用 index.html 内联注册）；不改 theme/style/App.vue；
- 不缓存 API/WS/视频；不请求通知权限直到用户点击；
- 不引入依赖（原生 Notification/Service Worker）。

## 验收

- build 产物齐全；localhost 下 SW 注册成功、离线 app shell 可用；
- 报告附 manifest/SW 注册证据与 build 体积。

## 执行报告

（待填：改动 / 注册/离线证据 / build / 遗留）
