# 任务 08 · Flutter 安卓客户端骨架【主力模型任务】

> 先读本目录 README.md。ADR-0003：客户端拒绝 WebView 套壳，主线 Flutter。

## 背景

M2 需要安卓客户端：看实时画面（任务01 的 WebRTC，可后接）、收告警（轮询 or WebSocket）、查事件。本任务只做**骨架竖切**：登录后（暂无鉴权，本地直连）事件列表页 + 详情（含截图）+ 设置页（服务器地址），网络层与状态管理定型，让后续功能往里填。

## 目标

新建 `android/`（Flutter 项目，`flutter create --org com.labsentinel labsentinel`）：

1. **依赖**：http、（WebSocket 用内置 dart:io）、provider（或 riverpod 二选一并说明理由）
2. **架构**：`lib/api/client.dart`（BaseURL 可配）、`lib/models/`、`lib/pages/{events, event_detail, settings}.dart`、`lib/main.dart`（底部导航）
3. **功能**：
   - 事件列表（分页、级别色条、下拉刷新）
   - 事件详情（截图全屏查看，复用服务端 `/api/events/{id}/snapshot`）
   - 设置页：服务器地址 + 连通性测试按钮 + WS 实时开关（开启则新事件弹系统通知，用 flutter_local_notifications）
   - 深色主题，配色对齐 Web 端（--bg #0f1419 等，见 web/src/style.css）
4. **构建**：`flutter build apk --debug` 产物路径写进报告（release 签名后续任务）
5. `android/README.md`：环境（Flutter SDK 版本）、构建命令、已知问题

## 验收标准

apk 在任意安卓设备/模拟器安装后：配置本机局域网地址能拉到真实事件列表、看截图、WS 开启时新事件通知。服务端无需改动。Web/服务端现有功能零影响。

## 执行报告（任务 08 · Flutter 安卓客户端骨架）

### 一、SDK 探查过程与结论

按任务要求在常见位置逐一排查 Flutter / Dart SDK：

1. `where flutter dart` → 均未找到（"用提供的模式无法找到文件"）；
2. 逐个 `ls` 确认常见安装位置：`D:\` 盘根、`C:\Program Files`、`C:\Program Files (x86)`、`D:\Program Files`、`D:\Program Files (x86)`、`C:\tools`（不存在）、`%LOCALAPPDATA%`、`%USERPROFILE%`、`D:\LabSentinel`、`D:\ZPACKAGE600`、`C:\` 根 → 均无 Flutter SDK；
3. PowerShell 全盘递归（深度 4）搜索 `flutter*` 目录 → 仅命中 WeType / FlClash / LocalSend / ziyoumao 等 Flutter 桌面应用自带的 `flutter_assets` **运行时产物，不是 SDK**；
4. 全盘搜索 `dart-sdk` 目录 → 无；
5. Android SDK：搜索 `adb.exe` / `sdkmanager.bat` → 仅 Escrcpy、i4Tools、Oppo Connect、UotanToolbox 等第三方工具**内置**的 adb，无 sdkmanager，即**无 Android SDK**；gradle 未安装、`~/.gradle` 无构建缓存；
6. Java 环境存在（Temurin JDK 25.0.3 与 JDK 21），但 Flutter SDK 本体缺失，无法构建。

**结论：本机无可用 Flutter SDK 与 Android SDK。** 按任务指示不尝试安装 SDK（体积大且网络受限），改为交付「可被 `flutter create .` 补全」的源码骨架，**构建被阻塞**（详见第四节）。

### 二、新建文件清单（全部位于 `android/`，未改动 server/ 与 web/）

| 文件 | 说明 |
|---|---|
| `android/pubspec.yaml` | 依赖清单（含状态管理选型注释） |
| `android/analysis_options.yaml` | flutter_lints 静态分析配置 |
| `android/lib/main.dart` | 入口：MultiProvider 注入 + 深色主题 + 底部导航 3 页（事件/实时占位/设置） |
| `android/lib/theme.dart` | 配色常量（逐项对齐 web/src/style.css 的 :root 变量）+ 全局主题构建 |
| `android/lib/app_state.dart` | AppState（ChangeNotifier）：WS 实时开关持久化 + 系统通知接线 |
| `android/lib/api/client.dart` | ApiClient：baseUrl 可配且持久化、fetchEvents/snapshotUrl/fetchSystem、dart:io WebSocket 监听 + 3 秒自动重连 + 按「最后事件 id」查重 |
| `android/lib/models/event.dart` | Event 模型 + fromJson（空安全兜底；兼容 REST 的 has_snapshot 与 WS 原始行） |
| `android/lib/services/notification_service.dart` | flutter_local_notifications 封装：初始化、Android 13+ 通知权限申请、事件通知渠道 |
| `android/lib/pages/events_page.dart` | 事件列表：下拉刷新、触底分页、左缘级别色条（L1/L2/L3 配色对齐）、点击进详情 |
| `android/lib/pages/event_detail_page.dart` | 事件详情：字段展示 + 截图（NetworkImage 带 loading/错误占位，点击进全屏 InteractiveViewer 缩放） |
| `android/lib/pages/settings_page.dart` | 设置页：服务器地址（自动补 http://、持久化）+ 测试连接（GET /api/system）+ WS 实时开关 |
| `android/README.md` | 环境要求、`flutter create --org com.labsentinel --project-name labsentinel .` 补全步骤、构建命令、清单必改项、已知问题 |

> 说明：页面文件按任务细则命名为 `events_page.dart` / `event_detail_page.dart` / `settings_page.dart`（任务文档正文的 `{events, event_detail, settings}.dart` 为简写）。另增 `theme.dart`（配色/主题共享）、`app_state.dart`（设置状态）、`services/notification_service.dart`（通知封装）三个支撑文件，均为 MultiProvider + ChangeNotifier 注入方案的组成部分。

### 三、状态管理选型理由

选 **provider**，弃 riverpod：

1. provider 由 Flutter 官方（flutter.dev）维护，与框架生命周期贴合；
2. 依赖极轻、无编译期代码生成步骤，`MultiProvider` + `ChangeNotifier` 的注入方式对骨架竖切完全够用（本骨架注入 ApiClient 与 AppState 两个 ChangeNotifier）；
3. riverpod 的编译期生成、多 ProviderContainer 架构对当前仅三个页面的规模属过度设计，后续真需要时再迁移。

### 四、构建状态：阻塞

**无法执行 `flutter build apk --debug`**：本机既无 Flutter SDK 也无 Android SDK（探查见第一节），且按任务铁律不允许安装。补全与构建需在装有 SDK 的机器上进行：

```bash
cd android
flutter create --org com.labsentinel --project-name labsentinel .   # 补平台文件
flutter pub get                                                     # 国内加 PUB_HOSTED_URL 镜像
# AndroidManifest.xml 加 usesCleartextTraffic="true" 与 POST_NOTIFICATIONS 权限（README 第三节）
flutter build apk --debug
# 产物：android/build/app/outputs/flutter-apk/app-debug.apk
```

依赖契约（pubspec 里的版本号在 `pub get` 时自动解析兼容）：http ^1.2.2（BSD-3）、provider ^6.1.2（MIT）、shared_preferences ^2.3.2（BSD-3）、flutter_local_notifications ^17.2.3（MIT），均为宽松协议，与项目 AGPL-3.0 兼容。

### 五、遗留问题

1. 服务端 `GET /api/events` 无 offset 参数，客户端用「累计 limit」模拟触底分页；事件量大后建议服务端加游标；
2. 开启 WS 时先拉最新 1 条做查重基线再建连，两者之间的事件可能漏通知（服务端无 since 参数，属结构性限制）；
3. WS 重连固定 3 秒间隔，无指数退避；
4. 「实时」页为占位，等任务 01（go2rtc/WebRTC）接入；release 签名、事件筛选 UI（client 已支持 level/camera_id 参数）为后续工作；
5. 构建被阻塞 → 未能在真机验收，首次 `flutter create .` 后如遇 SDK 版本差异（minSdk/compileSdk、插件主版本）按 android/README.md 第六节微调。
