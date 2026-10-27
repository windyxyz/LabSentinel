# 任务 37 · 移动端事件表格 375px 可读性修复（任务 34 遗留）

> 背景：任务 34 的移动基线发现：事件页在 375px 下虽然横向溢出为 0，但表格 9 列被压成"一字一行"竖排，可读性差。建议方向是给表格设 `min-width` 配合容器横向滚动。本任务只修这一个点。

## 目标

1. **修复 `web/src/views/Events.vue` 的表格移动端表现**（只改该文件 `<style scoped>`，必要时模板加一个 class）：
   - 375px 下不再出现"一个字一行"的竖排；
   - 推荐：给表格 `min-width: 720px`（或按实际列宽 760–820），外层滚动容器（现有）负责横向滚动；首列（时间）可 `position: sticky; left: 0` 提升可读性（注意背景色/边框与主题令牌一致）；
   - 桌面端（≥768px）保持现状，不改变列宽与布局。
2. **截图回归**：用 playwright 在 375×812、DPR 2 下打开事件页，截 `docs/assets/mobile/events-table_375.png`；同时确认页面级 `scrollWidth - clientWidth === 0`（溢出只在表格容器内部）。
3. **更新 `docs/assets/mobile/README.md`**：补这张图的检查点与复现步骤（一行即可）。
4. **执行报告**：写明改了什么、截图路径、桌面/移动两端验证结果。

## 约束

- 只动 `Events.vue` 的 scoped 样式与必要模板 class；**禁止**改 `theme.css`/`style.css`/`App.vue`/`main.js`；
- 不引入新依赖；颜色用现有令牌（`--card-bg/--stroke-*/--text-*` 等）；
- 用现有演示数据截图，不造脏数据；不启动 8000/5173 之外常驻服务（预览可用 5173，测完即可）。

## 验收标准

- 375px 下事件表可横向滚动、单元格文字不再逐字换行；页面无横向溢出；
- 桌面 1280px 下表头/列宽与修改前一致（截图或目检说明）；
- `npm run build` 通过；
- 新截图落盘 + mobile README 更新；
- 不改主线专属文件。

## 执行报告

（待填：改动文件 / 375px 与桌面验证 / build 结果 / 截图路径 / 遗留问题）

### 执行报告（flash，2026-10-28 00:58 交付）

- **改动文件**：
  - `web/src/views/Events.vue`（仅 scoped 样式 + 表格加 class）：`.events-table { min-width: 760px }`、
    `th/td { white-space: nowrap }`（杜绝一字一行）、首列时间 `position: sticky; left: 0`（背景
    `var(--card-bg, #fff)` 防透字，主题令牌优先）；未动主线专属文件，未引新依赖。
  - `tools/shot_events_375.py`（新）：playwright 回归脚本——页面级溢出断言 + `.panel` 容器滚动
    宽度输出 + 截图（导航用点击 `nav a`（无 vue-router），事件数据用 8001 测试 API + 临时 preview
    代理配置，用后即删）。
  - `docs/assets/mobile/`：`events-table_375.png`（修复后基线）、`events-table_1280.png`（桌面端不变证据）；README 已更新（任务 34 的"已知发现"标记已修复 + 回归方法）。
- **375px 验证**：页面溢出 **0px**；表格容器内可横向滚动（scrollWidth 800 > clientWidth 341）；
  列头/数据行横向铺开，无逐字竖排；时间列 sticky 常驻。截图含真实事件数据（1470 条中取首页）。
- **桌面 1280px 验证**：9 列完整布局与修复前一致（截图为证），min-width 不影响宽屏。
- **build**：`npm run build` 通过（gzip 42.77KB，487ms）。
- **遗留问题**：无。sticky 首列在暗色主题下的背景色依赖 `--card-bg` 令牌回退值，若主线后续调整
  主题令牌需同步核对（一行样式）。
