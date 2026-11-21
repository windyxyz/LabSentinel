# Release Notes · LabSentinel v0.3.0

> **未签名预览版**——签名证书未到位（T-403），安装时 SmartScreen 可能提示，
> 请核对 `SHA256SUMS-rust.txt` 校验和。
> 发行形态：Rust 单二进制 zip + Inno Setup 7 安装包（`LabSentinel-setup-0.3.0.exe`），
> 无 Python 运行时。完整变更见 `CHANGELOG.md` 的 `## [0.3.0]`。

## Highlights

- **检测模式三选一（UX2）**：设置中心「模型与检测」置顶分区——仅本地 /
  仅 API / **本地 + API 协作（推荐）**。协作模式只在规则命中或人员置信度进入
  灰区时才调用 API 精判，空场景零调用；每日预算耗尽自动降级本地并次日恢复；
  API Key 加密落盘、读取一律脱敏；远程地址双确认红线不降级。
- **导航 11 → 5（UX1）**：仪表盘 / 监控源 / 事件 / 告警 / 设置。图片识别与
  安全区并入监控源，报表并入仪表盘双视图，人脸/设备/模型迁入设置；旧地址
  自动重定向。
- **无黑框启动（UX3）**：双击不再弹控制台窗口；日志按天落盘
  `data/logs/server-YYYYMMDD.log`（保留 14 天）；致命启动错误弹原生提示框；
  `--console` 保留排障通道。
- **系统托盘（UX4）**：双击打开控制台，右键菜单含打开控制台 / 复制访问地址 /
  退出；退出走优雅关停（停任务 → WAL checkpoint → 退出），不再留下孤儿 ffmpeg。
- **GPU 默认开启**：推理后端缺省 DirectML（DX12 统一层），N 卡 / A 卡 / I 卡 /
  核显通用，初始化失败自动回落 CPU；仪表盘显示当前推理设备。
- **USB 摄像头开箱即用**：自动枚举本机 dshow 设备，不再手填索引；取流、
  快照、占用恢复全链打通。
- **告警推送可读、可去重、可带图**：中文人话文案；企业微信先文字后现场图；
  Webhook 带 `image_base64`；同一目标冷却窗口内静默，避免刷屏；渠道配置
  教程细化（官网链接/逐步操作/SMTP 失败对照），「发送测试」与实际派发同源。
- **安装包**：快捷方式选项（开始菜单/桌面）放在标准「选择附加任务」页（默认全选），
  完成页只保留「启动 LabSentinel」；安装 → 探活 → 卸载零残留已实测。安装版
  直接启动即可用：前端按安装根 `web\` 提供、数据落 `%LOCALAPPDATA%\LabSentinel\data`
  （Program Files 无需写权限）、模型随包自动启用。

## 升级须知

1. 数据库 schema 自动迁移 v13 → v14（events 新增 VLM 复核字段），升级前建议
   关闭正在运行的服务；迁移幂等，失败不会破坏原库。
2. 旧 `.env` 中的检测相关变量仍作为兜底读取，但网页上保存的配置优先。
3. 若此前手动固定 `LS_RUNTIME_EP=cpu`，升级后仍保持 CPU；删除该变量即恢复
   默认的 DirectML 优先。
4. 安装包默认「仅为我安装」到 `%LOCALAPPDATA%\Programs\LabSentinel`；数据目录
   `%LOCALAPPDATA%\LabSentinel\data` 不随卸载删除。

## 验证基线（2026-11-21）

| 项 | 结果 |
|---|---|
| Rust workspace | 134 passed |
| Web（vitest） | 52 passed |
| 前端构建 | 成功 |
| cascade 验收（acceptance_vlm） | 14/14 |
| Rust-only 验收（acceptance_rust_only） | 39/39 |
| 路由覆盖 | 106/106 = 100% |
| 实体机实测 | USB 枚举/取流、托盘、无黑框、GPU DirectML、告警带图+冷却、白大褂 4 图 e2e 通过 |

## 已知问题

1. **PPE 模型对本批灰度监控图检出为 0**：模型/阈值调优项，事件与告警链已用
   occupancy 规则独立验证通过。
2. **未签名**：SmartScreen 提示属预期；请核对 SHA256。
3. 真实 RTSP 24 小时挂机验收仍在排期。
