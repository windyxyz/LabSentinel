# 任务 66 · 前端加载/错误/空状态统一

> 背景：各页面在接口慢、失败、无数据时表现不一致：有的空白、有的只 console.error、有的表格空着。
> 新用户遇到服务未启动/令牌错误时很难判断发生了什么。本任务统一四类页面的状态呈现（不引入 UI 框架）。

## 目标

覆盖 `web/src/views/` 下 **Dashboard、Cameras、Events、Reports** 四页（Face 页可顺带，不强制）：

1. **加载态**：首次请求期间显示「加载中…」占位（文本或简单 skeleton），不要闪现空表格；
2. **错误态**：请求失败显示红色提示条，包含 `error.message`（已有中文前缀）+「重试」按钮；
   401 时提示去 Cameras 页填管理令牌（沿用 `api.js` 现有文案）；
3. **空状态**：请求成功但无数据时显示可操作提示，例如：
   - Dashboard：暂无事件 →「去摄像头页添加/启动摄像头」；
   - Events：当前筛选无事件 →「清除筛选」按钮；
   - Reports：所选区间无数据 → 提示换日期；
   - Cameras：暂无摄像头 → 指向下方添加表单。
4. **刷新不闪**：轮询/定时刷新失败时保留上一份数据，只显示一条「刷新失败，正在重试」提示，
   不要把已有内容清空（Dashboard 每 5s 轮询、Events 轮询需检查现状后处理）。
5. **按钮态**：提交/删除/启动等异步按钮在请求期间 `disabled` + 文案变化，防重复点击
   （至少覆盖 Cameras 添加/启动/删除、Events 状态操作）。

实现建议：在 `web/src/components/` 新增一个 `AsyncState.vue`（props: loading/error/empty/emptyText，
slot 默认内容 + retry 事件），四页复用它；也可用每页局部实现，但**不要**动 `App.vue/main.js/theme.css/style.css`。

## 测试/验收

- `cd web && npm run build` 通过；gzip 体积增长合理（记录数值，不超过 +10KB gzip）。
- 手工/脚本验证（可用现有 playwright 工具或直接起 8001 + vite）：
  - 停掉服务端 → Dashboard 显示错误条 + 重试按钮，页面不白屏；
  - 服务端恢复 → 点重试后数据回来；
  - 空库（临时 data）→ 四页空状态文案正确；
  - 375px 宽度下错误条/按钮不溢出（截图存 `docs/assets/mobile/` 或临时目录，报告写路径）。
- 不要求新增自动化 E2E；但需在报告列出实际验证步骤与结果。

## 约束

- **禁止修改** `web/src/theme.css`、`web/src/style.css`、`web/src/App.vue`、`web/src/main.js`；
- 不引入 Element/Ant/Vuetify 等 UI 框架；样式写组件 scoped 或复用现有 class；
- 不改变 API 契约；不改后端（除非发现 401 文案不一致，可在 `api.js` 内修）；
- 若某页已有完善状态处理，不要重写，只补齐缺口并在报告说明。

## 验收标准

- 四页均有加载/错误/空三态；轮询失败不清空旧数据；异步按钮防重复；
- build 通过、体积记录；375px 无横向溢出；
- 报告附每页验证结果与改动文件清单。

## 执行报告

（待填：改动文件 / 四页验证 / build 体积 / 遗留）
