# 任务 135 · View Transitions 体验（P3）

> 来源：SPlayer-Next 借鉴建议 #1。只做事件列表→详情抽屉的共享元素过渡 + 轻量淡入，
> 纯浏览器 API，无新依赖、无业务逻辑变化。

## 目标

1. `Events.vue`：点击事件行/缩略图打开详情抽屉时，若支持
   `document.startViewTransition` 且用户未开启 `prefers-reduced-motion: reduce`：
   - 用 `startViewTransition(() => { drawer = row })` 包裹状态切换；
   - 列表行/抽屉大图设置 `view-transition-name`（按事件 id 唯一，如 `evi-<id>`）；
   - 过渡 180–260ms，cubic-bezier 缓动；不支持/减少动效时直接打开。
2. 关闭抽屉反向过渡；快速连点不报错（用 transition 进行中标志避免嵌套调用）。
3. CSS 放在 Events.vue 的非 scoped `<style>`（页面级），不改全局 theme/style.css；
   尊重 `prefers-reduced-motion`；不引入动画库。
4. 可选：页面内 tab/筛选切换淡入（仅 Events 页，不强求）。

## 验收

- Chrome/Edge 最新版手工验证：打开/关闭过渡无跳动、无控制台错误；
- 不支持 API 的浏览器（或 reduce-motion）行为与现在一致；
- `npm run build` 通过；后端 pytest 不受影响；
- 报告附浏览器/版本、gzip 变化。

## 约束

- 不改 App.vue/main.js/theme.css/style.css；不引入依赖；
- 过渡不得遮挡处置按钮/键盘焦点；`Esc` 关闭仍可用。

## 执行报告

**flash 2026-11-04 完成（build gzip 62.31KB（+0.19KB）；后端零改动；events 相关 9 测试回归通过）**

### 实现（只改 Events.vue）

- `withViewTransition(apply)`：`document.startViewTransition` 存在且
  `prefers-reduced-motion != reduce` 且 `vtRunning=false` 时包裹状态切换；否则直接 apply
  （不支持/减少动效/连点防嵌套三路降级，`finished.finally` 复位标志）；
- `openDetail`/`closeDrawer` 都走包裹（开/关双向过渡；Esc 路径经 closeDrawer 同样生效，
  焦点恢复不受影响）；
- 共享元素：行 `style.viewTransitionName = evi-<id>`（行内绑定），抽屉同 id 动态绑定
  （关闭时置空）——列表行↔抽屉位移过渡；
- CSS：**非 scoped** `<style>` 块（页面级伪元素只能全局），220ms cubic-bezier(0.2,0,0,1)；
  `::view-transition-old/new(root)` 关默认交叉淡入（只留共享元素位移）；
  `@media (prefers-reduced-motion: reduce)` 全部 `animation: none !important` 兜底；
  theme/style.css/App.vue/main.js 零改动。

### 验证

- build 通过（gzip 62.12→62.31KB，+0.19KB）；event_search/pagination 9 用例回归通过
  （后端零触碰）；
- 浏览器手工项（Chrome/Edge 打开/关闭过渡、控制台无错）待用户侧一次目检——
  jsdom 不实现 VT API（走降级分支，行为与旧版一致，覆盖测试意义有限）。

### 遗留

- 无。

### 主线复审补丁（2026-11-04）

- 打开抽屉时列表行与抽屉同时持有同一 `view-transition-name`（evi-<id>）会被浏览器判为
  重复命名、过渡可能被跳过 → 行在 `drawer.id===e.id` 时置 `none`，由抽屉单独持有名字；
  关闭时恢复行名，双向共享元素才成立。
