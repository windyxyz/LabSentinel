# LabSentinel 安卓客户端（Flutter 骨架）

> 任务 08 交付物。本目录是一个**待补全的 Flutter 工程**：`lib/` 业务源码与 `pubspec.yaml` 已就绪；
> 平台原生文件（`android/`、`ios/` 等子目录）需在本机装好 Flutter SDK 后用 `flutter create .` 补全。
> 交付机器上没有 Flutter / Android SDK，构建被阻塞（探查结论见 历史任务包（已归档删除） 执行报告）。

## 一、环境要求

| 项 | 要求 |
|---|---|
| Flutter SDK | stable 通道，≥ 3.22（Dart ≥ 3.4，满足 pubspec 的 sdk 约束） |
| Android SDK | Platform-Tools + 对应 compileSdk（`flutter doctor` 全绿为准） |
| JDK | 17+（交付机器已有 Temurin JDK 21 / 25，可直接用） |
| 服务端 | 局域网内运行：`cd server && ..\.venv\Scripts\python.exe -m uvicorn app.main:app --port 8000`（`.venv` 由 `python scripts\deploy.py install` 创建；详见 [部署指南](../docs/部署指南.md)） |

## 二、初始化与构建

```bash
cd android
# 1) 补全平台文件（--project-name 指定包名 labsentinel，
#    避免目录名 android 被当成项目名生成 com.labsentinel.android 这种包名）
flutter create --org com.labsentinel --project-name labsentinel .

# 2) 安装依赖（国内网络建议加镜像环境变量）
#    export PUB_HOSTED_URL=https://pub.flutter-io.cn
#    export FLUTTER_STORAGE_BASE_URL=https://storage.flutter-io.cn
flutter pub get

# 3) 按下面「创建后必改」调整 AndroidManifest.xml

# 4) 构建 debug apk（release 签名属后续任务）
flutter build apk --debug
# 产物：build/app/outputs/flutter-apk/app-debug.apk
```

> `flutter create .` 只补缺失的平台文件，**不会覆盖**已有的 `lib/`、`pubspec.yaml`。

## 三、创建后必改（android/app/src/main/AndroidManifest.xml）

1. **明文流量**（局域网 http 直连必需，Android 9+ 默认禁止）：
   `<application>` 标签加 `android:usesCleartextTraffic="true"`
2. **通知权限声明**（Android 13+，flutter_local_notifications 运行时申请前需先声明）：
   `<manifest>` 下加 `<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>`

## 四、服务端地址配置

- 首页为事件列表；首次使用到「设置」页填写服务端地址（形如 `192.168.1.10:8000`，
  自动补 `http://` 前缀），点「测试连接」验证（走 `GET /api/system`），保存后持久化于 shared_preferences。
- 手机与服务器必须**同一局域网**；真机上**不能填 localhost / 127.0.0.1**（那是手机自己），
  要填电脑的局域网 IP（`ipconfig` 查看）。
- WS 地址由 http 地址自动推导：`http://host:port` → `ws://host:port/ws/events`。
- 设置页的「WS 实时事件推送」开关持久化；开启后新事件弹系统通知
  （Android 13+ 首次开启会弹通知权限对话框，拒绝后收不到通知需到系统设置里手动允许）。

## 五、目录结构

```
android/
├── pubspec.yaml                       # 依赖：http / provider / shared_preferences / flutter_local_notifications
├── analysis_options.yaml              # flutter_lints
└── lib/
    ├── main.dart                      # 入口：MultiProvider 注入 + 深色主题 + 底部导航 3 页
    ├── theme.dart                     # 配色常量（对齐 web/src/style.css）与全局主题
    ├── app_state.dart                 # AppState：WS 开关持久化 + 通知接线（ChangeNotifier）
    ├── api/client.dart                # ApiClient：REST + WS 自动重连 + baseUrl 持久化 + 事件查重
    ├── models/event.dart              # Event 模型 + fromJson（对齐服务端 events 表）
    ├── services/notification_service.dart  # flutter_local_notifications 封装（Android 渠道/权限）
    └── pages/
        ├── events_page.dart           # 事件列表：分页 / 级别色条 / 下拉刷新
        ├── event_detail_page.dart     # 事件详情：字段 + 截图（点击全屏缩放）
        └── settings_page.dart         # 服务器地址 / 测试连接 / WS 开关
```

## 六、已知问题

1. **分页**：服务端 `GET /api/events` 暂无 offset 参数，客户端用「累计 limit」模拟触底分页；
   事件量很大后建议服务端加游标参数。
2. **漏通知窗口**：开启 WS 时先拉最新 1 条事件作为查重基线再建连，两者之间产生的事件可能漏通知。
3. **重连策略**：WS 断线固定 3 秒重连，无指数退避；弱网下可再优化。
4. **实时画面页为占位**，等任务 01（go2rtc / WebRTC）就绪后接入。
5. **依赖版本**：`flutter_local_notifications ^17.2.3` 需 minSdk ≥ 21（Flutter 默认模板满足）
   与 compileSdk 34（近期 Flutter 模板默认满足）；若 pub 解析到更高主版本，按其 changelog 调整。
6. release 签名 / 统计页 / 事件筛选 UI（level/camera 过滤客户端已支持，UI 未做）为后续任务。
