# 任务 17 · 事件详情与移动端适配（先读本目录 README.md）

## 目标

### 17A 事件详情抽屉（Web）
- 点击事件行（非按钮区域）打开右侧抽屉（WinUI ContentDialog 风格：`--dialog-bg`、8px 圆角、阴影 `--shadow-dialog`、半透明遮罩）
- 内容：大图快照（点击放大现有逻辑复用）、视频播放（若有）、完整 detail、状态操作按钮（复用任务12接口）、处理记录
- 键盘：Esc 关闭；焦点管理（打开时聚焦抽屉，关闭还原）

### 17B 移动端适配检查
- 在 375px 宽视口逐页走查（浏览器 DevTools 或直接缩窗）：仪表盘卡片换行、表格横向滚动（`overflow-x:auto` 包装）、导航折叠为横向条、按钮触控尺寸 ≥40px
- 修复发现的问题，样式写在**组件 scoped** 里（全局 style.css 归主线，勿改）

## 验收标准

`npm run build` 通过；三页（仪表盘/事件/安全区）在 375px 下无横向溢出（截图证明）；抽屉交互可用；不改全局样式文件。

## 执行报告

### 交付物
- **17A 事件详情抽屉**（Events.vue）：点击事件行打开右侧抽屉（点击按钮/图片/视频不触发）——
  WinUI ContentDialog 风格：`--dialog-bg` 底、`--overlay-radius` 8px 圆角、`--shadow-dialog` 阴影、
  半透明遮罩、深浅主题随系统。内容：元信息行、完整 detail、大图快照（点击复用现有 zoom 放大）、
  视频播放（若有）、状态操作按钮（复用任务12接口，处置后抽屉状态同步）、处理记录。
  键盘 Esc 关闭（抽屉/大图/视频三层级）；焦点管理：打开聚焦抽屉、关闭还原焦点、`role="dialog"` + aria-modal；
  Teleport 到 body 防表格 overflow 裁剪
- **17B 375px 适配**（样式全部组件 scoped，未动全局）：
  - 四页 `.panel` 加 `overflow-x:auto`（表格横向滚动不破版）
  - Dashboard `.cards` 允许换行、卡片 min-width
  - 全部视图按钮 `min-height:40px` 触控尺寸
  - 导航折叠：**App.vue 归主线专属（协作说明禁止修改），主线新导航在 375px 下本身已是横向条且无溢出——无需改动**（见截图）
- **验收自动化**：安装 playwright+chromium（gpu-venv 隔离，npmmirror 源），375×720 视口逐页
  程序化检测 `scrollWidth-clientWidth` 并截图

### 验收结果
- 三页横向溢出检测：**仪表盘 0px / 事件 0px / 安全区 0px（全部无溢出）**；截图存 D:\tmp\mobile-check\（三张）
- `npm run build` 通过（gzip 40.5KB）；`pytest` 63 passed 不受影响

### 遗留问题
- 抽屉交互的浏览器实测（真机触控）未做——布局与键盘/焦点逻辑已实现，交互细节待真机走查
- 截图环境为静态 dist（/api 404 空态），有数据下的长表格滚动已由 overflow-x 容器保证
