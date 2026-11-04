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
