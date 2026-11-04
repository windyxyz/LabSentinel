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

## 执行报告

**flash 2026-11-04 完成（npm test 26/26 passed / 2.66s；npm run build 62.12KB 不变；CI 已加 npm test）**

### devDependencies（不加生产依赖；bundle 体积零变化）

- `vitest@^2.1.9`、`jsdom@^29.1.1`、`@vue/test-utils@^2.5.1`（package.json `test: vitest run`）；
- **兼容性坑（已记录）**：最新 vitest 5.1 要求 vite ≥6 的 `./module-runner` 导出，本项目
  vite 5.4 → 安装即 Startup Error `ERR_PACKAGE_PATH_NOT_EXPORTED`，**降级 vitest@^2.1.9**
  （vite5 兼容矩阵）；npm i 需 `--legacy-peer-deps`（vue peer 声明冲突）；
- vite.config.js 加 `test: { environment: 'jsdom', include: ['tests/**/*.test.js'] }`
  （单配置文件，vite 复用；测试零网络/零真实 Notification）。

### 用例（web/tests/，26 个）

- `api.test.js`（14）：token 存取、mediaUrl ?/& 拼接与 encodeURIComponent、2xx JSON、
  非 2xx detail 抛错、401 引导文案、X-Admin-Token/Content-Type/body 断言、
  URL 拼装抽检（usb/refresh/groups/plugins/enable/stop/group/logs tail）；
- `notify.test.js`（7）：default 不弹、环境不支持不抛、L1 不弹、tag 格式、页面可见不弹、
  **同 tag 10s 去重（fake timers 10_001ms 后可再弹）**、开关 localStorage、
  ensurePermission 两分支；
- `asyncstate.test.js`（5）：loading/error（retry 事件+401 提示区分）/empty/slot+notice。

### CI

`.github/workflows/tests.yml` web-build job：`npm ci → npm test → npm run build`
（YAML 校验过；runner 实跑待远端，发布阻塞项不变）。

### 遗留

- 无。vitest 升 vite6 时可回 vitest@5（记录在案）。
