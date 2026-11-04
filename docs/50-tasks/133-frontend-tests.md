# 任务 133 · 前端自动化测试（vitest，P2）

> 现状：后端 pytest 491，前端只有 `npm run build`；每次 UI 改动（插件/AI/时间线）只能靠手工。
> 本任务建立最小但可扩展的前端测试基线，**不加生产依赖**。

## 目标

1. 引入 devDependencies：`vitest`、`jsdom`、`@vue/test-utils`；`package.json` 加 `"test": "vitest run"`。
2. 测试（`web/src/**/__tests__` 或 `web/tests/`）：
   - `api.js`：成功 JSON、非 2xx 错误体 detail、401 提示、`mediaUrl` 有/无 token 的 `?`/`&` 拼接；
   - `notify.js`：权限未授予不弹、L1 不弹、L2/L3 tag 去重、开关关闭不弹（mock Notification/localStorage）；
   - `AsyncState.vue`：loading/error/empty/slot 四分支 + retry 事件；
   - `api.folderImages` / `plugins` 等新增方法 URL/参数拼装（mock fetch）。
3. `vite.config`/`vitest.config` 配 jsdom 环境；测试不访问网络/真实 Notification。
4. `.github/workflows/tests.yml` 的 frontend job 增加 `npm test`。

## 验收

- `npm test` 全绿（≥15 用例）；`npm run build` 通过；无生产依赖/bundle 体积变化（devDeps 不计）。
- 报告附用例数、耗时、CI 配置 diff；若 jsdom 与 Node 版本冲突，记录并降级为 node 环境纯函数测试。

## 约束

- 只测纯逻辑/组件契约，不引入 Playwright/E2E 浏览器（P3 另议）；
- 不修改 theme/App.vue/main.js；测试文件不影响构建产物。
