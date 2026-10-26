# 客户端采用原生渲染框架（Flutter 方向），拒绝 WebView 套壳

---
status: accepted
date: 2026-10-25
---

用户对客户端定下硬原则：**不用 WebView 套壳**（Electron / Tauri / uni-app 经典版等，"直接用网页不好吗"）。第一轮调研的 uni-app x 方案虽然编译原生、可一套代码覆盖五端，但其工具链（HBuilderX/编译器）闭源成分多、成熟度对新项目有风险，且用户价值排序已是"原生体验 > 端数覆盖"。因此客户端主线改为 **Flutter**：Android(M2) → Windows/macOS/Linux 桌面 + iOS(M4)，全部原生渲染、单一 Dart 代码库。

## Considered Options

- **uni-app x**（否）：五端覆盖最强（唯一官方支持鸿蒙+小程序），但违背"拒绝套壳/闭源工具链"偏好，且复杂视频播放与推送能力未验证。
- **Compose Multiplatform / Kotlin 原生**（备选）：同为原生渲染；JetBrains 系，桌面端成熟。M2 启动 Android 时与 Flutter 最终二选一。
- **鸿蒙端**：Flutter OHOS 为社区移植，成熟度届时评估，不达标则放弃该端（不为它动摇主线）。
- **微信小程序**（放弃或独立做）：与 Flutter 无代码共享；确有需要时用 uni-app 单独实现小程序端，不作为主客户端。

## Consequences

- 失去"一套代码出小程序"的能力——小程序降级为可选独立实现。
- Web(M1) 与 Flutter(M2+) 是两套前端（Vue3 vs Dart），管理后台保持 Web 独占；移动/桌面客户端页面用 Flutter 重写。
