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

**flash 2026-11-04 完成（build gzip 59.56KB；dist 产物齐全；8188 实机 9 项断言全 PASS）**

### 1. PWA

- `web/public/icons/icon-{192,512}.png`：cv2 绘制主题蓝底白盾+摄像头眼（2.6KB/7.9KB；
  **坑**：cv2.imwrite 不吃中文路径 → imencode+write_bytes）；
- `web/public/manifest.webmanifest`：standalone/start_url / /theme #1b3366/双尺寸图标；
- `web/public/sw.js`：install 缓存 app shell（`/`、index、manifest、icons）+skipWaiting；
  activate 清旧版本；fetch 策略——**导航 network-first（离线回退缓存 index）、/assets/*
  cache-first（hash 文件）、`/api/*` 与 `/ws` 一律直连不缓存**（version 常量 v1，更新 bump）；
- `web/index.html`：manifest link + theme-color + apple-touch-icon + **内联 module script
  注册 `/sw.js`（仅 http/https）——main.js 零改动**（无需看板申请，规格即此要求）。
  **发现**：Vite build 会把内联 module 抽进主 chunk（代码仍在，`register("/sw.js")` 实测在
  产物末尾）——实机断言按 chunk 内容校验。

### 2. 浏览器通知（web/src/notify.js 新文件 + Dashboard.vue）

- `ensurePermission()`（只在用户点击时请求）、`notifyEvent(ev)`（`document.hidden` 才弹、
  **L1 不通知**、tag=`cam-<id>-<type>` 同 tag 10s 去重、onclick focus、失败静默）；
  开关 localStorage `LS_DESKTOP_NOTIFY`（0=暂停）；
- Dashboard.vue：卡片行新增「桌面通知」卡（未授权→"开启桌面通知"按钮；已授权→勾选开关，
  权限状态 tooltip）；现有 WS `onEvent` 内接 `notifyEvent`；路由零新增、全局四件套未动。

### 3. 验收证据（8188 临时实例，dist 挂载）

9 项断言全 PASS：index 含 manifest link/theme-color、**SW 注册代码在构建产物 chunk 内**、
`/manifest.webmanifest` `/sw.js` `/icons/*.png` 全 200、sw.js 明确排除 /api/ 缓存、
manifest 字段（standalone/start_url/双图标）。build：index-*.js 159.06KB（gzip 59.56KB）。

### 4. 文档

部署FAQ「怎么把面板装成应用/收桌面通知（需 localhost 或 HTTPS）」；运维手册 §5.1 值班桌面通知。

### 遗留

- ①**浏览器真机 SW 注册/离线回退**（localhost 打开→开发者工具 Application→SW activated→
  断网刷新出壳）需一次人工/Playwright 验证——curl 只能证明资产可达与注册代码在产物中；
- ②dist/assets 堆积多代旧 hash chunk（121 时代的 index-9gf8y74S.js 等）——构建未清空
  outDir（emptyOutDir 表现异常，非本任务引入），影响仅体积；建议 129 发布卫生处理；
- ③SW 版本常量 v1 与构建 hash 无关，未来改 shell 清单时 bump。
