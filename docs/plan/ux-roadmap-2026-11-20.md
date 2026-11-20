# 桌面体验与界面简化路线图（UX 轮）

> 创建：2026-11-20。定位：**设计文档（设计者视角，不直接开发）**。
> 用户诉求三条：① 简化操作界面，相关功能整合进设置中心（以「模型相关」为首个样板，
> 检测模式提供三种：仅本地 / 仅 API / 本地+API 协作——本地先扫到人与可疑，再把
> 相关图片传 API 精判，节省 API 费用）；② 程序启动不弹黑框；③ 任务栏托盘便于
> 访问、管理、退出。
> 执行纪律沿用 rust-first-roadmap：每阶段一个或多个 commit，门禁全绿才推进，
> 进度打勾记录在本文件。

## 0. 现状盘点（事实与出处）

| 事实 | 出处 |
|---|---|
| 主导航 11 个一级入口：仪表盘/摄像头/图片识别/安全区配置/事件查询/报表/告警配置/人脸管理/模型管理/设备管理/设置 | `web/src/App.vue` tabs |
| 设置中心已有 6 分区 + 侧栏搜索（令牌/通知/数据/文案/插件/服务） | `web/src/views/Settings.vue` |
| VLM（外部联网模型）配置**只认环境变量**：LS_VLM_PROVIDER/URL/MODEL/API_KEY/TIMEOUT_S/ALLOW_REMOTE/ENABLED——网页端不可配 | `engine-rs/crates/ls-server/src/vlm.rs:34-47` |
| 相机级检测后端是二选一：rust（本地）/ vlm，且历史值 python 一律映射为 rust | `Cameras.vue analyzerOf()`；`handlers/cameras.rs` 校验 `python\|rust\|vlm` |
| 启动链：`LabSentinel.bat` 双击 → cmd 黑框常驻 + 控制台日志；`--launch` 已有端口试探防双实例 + launcher.lock + 就绪开浏览器 | `scripts/build_rust.py` launcher；`main.rs` |
| 优雅关停通道已存在：`POST /api/system/shutdown` → `shutdown_server`（网页「服务控制」用） | `router.rs:133,247` |
| 配置存储先例：`app_settings` 表 upsert/prefix/delete + Fernet 兼容加密（`enc:v1:` 前缀，告警渠道凭据在用） | `ls-store` settings 族；`ls-server/src/fernet.rs` |
| 事件表已有 VLM 判定回填列（RA5 验收加的 verdict 族） | `ls-store` events verdict |
| 视频路 VLM 限流先例：每路 6 次/分钟 | vlm.rs/worker 路径 |
| 兼容红线先例：VLM URL 非回环需显式放行（防误外发截图） | `LS_VLM_ALLOW_REMOTE` |

## 1. 设计原则

1. **设置中心是「管理面」，主导航只留「使用面」**——低频管理功能（模型/设备/
   人脸/插件/令牌）收进设置；高频操作（看板/监控/事件/告警）保持一级入口。
2. **检测模式是全局策略 + 相机级覆盖**，默认值只有一份事实源（DB），env 只做
   首次引导与兜底，不再要求用户改环境变量。
3. **省钱是 cascade 模式的第一验收指标**——「无人/无触发帧不产生 API 调用」
   必须有可断言的实证（mock VLM 调用计数）。
4. **桌面体验零惊讶**：无黑框不等于无反馈——日志必须文件化，致命错误必须
   有可见出口（弹窗/日志双通道）；托盘是唯一常驻 UI。
5. **不破坏既有契约**：`acceptance_rust_only` 39/39 与 `acceptance_vlm` 不回退，
   新能力以增量 API + 扩展取值落地。

---

## UX1 信息架构简化（导航 11 → 6，设置中心扩容）

### 方案对比

| | 方案 A（保守） | **方案 B（推荐）** |
|---|---|---|
| 主导航 | 7：仪表盘/摄像头/图片识别/事件/告警/人脸/设置 | **6：仪表盘/监控源/事件/告警/设置 + 仪表盘内报表** |
| 安全区配置 | 独立 tab 保留 | 并入「监控源」详情步骤（zone 本就 per-camera） |
| 图片识别 | 独立 tab | 并入「监控源」（文件夹源=第二种源类型） |
| 报表 | 独立 tab | 并入仪表盘「今日概览/报表」双视图 |
| 人脸管理 | 独立 tab | 并入设置「人脸识别」 |
| 模型管理/设备管理 | 并入设置 | 并入设置（同左） |

推荐 B：一级入口数量减半且按「使用者心智」分组；A 作为回退形态（若「监控源」
合并验收不顺，可只回退该项，其余不变）。

### 设置中心最终分区（搜索已内置，9 分区）

| 分区 | 内容 | 来源 |
|---|---|---|
| **模型与检测**（新，置顶） | 检测模式三选一 + 本地模型 + API 服务 + 协作策略 | 吸收「模型管理」页 + 相机 analyzer 默认值职责（见 UX2） |
| 通知与告警 | 渠道/测试/静默 | 现有 |
| 告警文案 | 模板/预览 | 现有 |
| 设备与配对 | 设备表/配对码 | 迁入「设备管理」页 |
| 人脸识别 | 名册/同意/合规门 | 迁入「人脸管理」页 |
| 数据管理 | 导入导出/备份 | 现有 |
| 插件 | 上传/启停/日志 | 现有 |
| 服务控制 | 停止/重启/**日志尾部（新增）** | 现有 + UX3 |
| 管理令牌 | 令牌管理 | 现有 |

### 落地要点

- 前端重组为主，后端 API 基本不动（UX2 除外）；旧 tab 路由做 replace 重定向，
  不允许 404（外链/书签兼容）。
- 相机编辑里的 analyzer 下拉改为「**跟随全局（默认）** / 仅本地 / 仅 API /
  本地+API 协作」——把「每台相机单独设」升级为「全局定策略、异常相机单独覆盖」。
- Web 测试同步更新（44 例基线不回退，新增导航断言）。

### 验收

- [ ] 主导航 6 入口；设置 9 分区侧栏搜索全部可达
- [ ] 旧路由访问自动跳转新位置（无 404）
- [ ] Web 测试 + build 全绿；acceptance_rust_only 39/39

---

## UX2 检测模式三选一（local / api / cascade）——核心

### 2.1 数据模型（app_settings 新键，fernet 加密先例）

```
detect.mode             = local | api | cascade      # 全局默认，缺省 local
detect.vlm.provider     = openai_compat | ollama | gemini
detect.vlm.url          = https://…                 # 明文
detect.vlm.model        = gpt-4o-mini
detect.vlm.api_key      = enc:v1:…                  # fernet；GET 一律脱敏 ***
detect.vlm.timeout_s    = 30
detect.vlm.allow_remote = true|false                # 合规确认位（见 2.4）
detect.cascade.trigger  = persons | rules | both    # 缺省 both
detect.cascade.conf_lo / conf_hi = 0.35 / 0.65      # 本地置信度灰区复核带（P2 可选）
detect.cascade.daily_budget = 200                   # 每日 API 复核上限，0=不限
detect.cascade.on_negative = keep | suppress        # VLM 否定时：照常投递/抑制，缺省 keep
detect.usage.date / detect.usage.count             # 当日调用计数（跨重启累计）
```

- 读取顺序：**DB → env → 默认**（env 降级为首次引导/CI 兜底）。
- `cameras.analyzer` 取值扩展：`rust | vlm | cascade | default`（default=跟随全局；
  历史 python→rust 映射已有）。`set_camera_analyzer` 校验串同步扩展。

### 2.2 引擎语义（cascade 是新增路径，local/api 复用现有）

```
每帧 → 本地 ONNX 推理（三种模式下都照常跑，零 API 成本）

mode == api      ：判定直接走 VLM（现有 vlm 路径，person/违规由 API 结论驱动事件）
mode == cascade  ：本地先行 ——
    触发 = (persons ≥ 1 或 规则命中) 且 置信度不在灰区外沿（可选收紧）
    触发 且 限流通过 且 今日预算未耗尽：
        取该帧/证据截图 → 异步复核队列（不阻塞帧循环，复用每路 6 次/分钟限流）
        VLM verdict 回填 events.verdict 族（列已存在）
        on_negative == suppress 且 VLM 明确否定 → 事件标记 vlm_suppressed=1 不投递
    预算耗尽 → 自动降级 local-only + 设置页横幅 + 事件标 unreviewed
```

- 复用件：folder 单图 VLM 判定路径、视频 worker 证据环抽帧、事件 verdict 回填、
  mock_vlm_server.py（验收替身）。
- **省钱实证**：acceptance_vlm 扩展断言——空场景帧序列跑 cascade，mock VLM
  **调用计数必须为 0**；有人帧序列调用计数 > 0 且 verdict 回填正确。

### 2.3 API 变更（新增三条，不改既有契约）

| 路由 | 语义 |
|---|---|
| `GET/PUT /api/detect/config` | 读写 detect.*；GET 的 api_key 脱敏；PUT 空串=不变更 |
| `POST /api/detect/test` | 用当前配置发一张测试图（内置样张或最近一帧）→ 判定/耗时/错误（「测试连接」按钮） |
| `GET /api/detect/usage` | 今日 API 调用数 / 预算余量（设置横幅 + 仪表盘卡片共用） |

`/api/system` 的 model_status 扩展 `mode` 与 vlm 连通态。

### 2.4 UI（设置 → 模型与检测）

```
┌ 检测模式 ───────────────────────────────────────────┐
│  ┌──────────┐  ┌──────────┐  ┌────────────────────┐ │
│  │ 仅本地    │  │ 仅 API   │  │ 本地 + API 协作 ⭐  │ │ ← 卡片单选
│  │ 零成本离线│  │ 全量精判 │  │ 省费用的推荐模式    │ │
│  └──────────┘  └──────────┘  └────────────────────┘ │
│  [本地模型]  [API 服务]  [协作策略]   ← 随模式显隐    │
└────────────────────────────────────────────────────┘
```

- 本地模型卡：模型选择/上传（吸收原「模型管理」页）、置信度阈值、PPE 模型。
- API 服务卡：provider 下拉、base_url、api_key（密码框）、模型名、超时、
  **测试连接**按钮；远程 URL 时必须勾选「我确认允许将截图发送到该外部服务」
  （写 `detect.vlm.allow_remote`，与 env 红线**任一为真才放行**，双确认不降级）。
- 协作策略卡：触发条件、每日预算、VLM 否定策略；预算余量实时显示。

### 验收

- [ ] 三模式切换**即时生效**（PUT 后相机 worker 软重启对齐现有 analyzer PUT 行为）
- [ ] cascade 省钱实证：空场景 0 次 API 调用（mock 计数断言）
- [ ] 预算耗尽自动降级 + 横幅；计数跨重启累计
- [ ] api_key GET 脱敏、不写日志；远程未确认 → 拒绝调用并落 error 事件
- [ ] acceptance_vlm 扩展后全绿；acceptance_rust_only 39/39

---

## UX3 无黑框启动 + 日志文件化

### 3.1 方案

- `main.rs` 顶部：`#![cfg_attr(all(windows, not(debug_assertions), windows_subsystem = "windows")]`
  ——release 双击无控制台；debug 保留控制台便于开发。
- `--console` 显式参数：release 下 `AttachConsole(ATTACH_PARENT_PROCESS)`，
  命令行启动仍可看输出（排障保留通道）。
- **GUI 子系统进程在父进程用管道重定向时 stdout 仍然有效**——CI 冒烟/smoke_rust/
  安装器 `--selftest-http` 依赖 exit code 与管道输出，不受影响。此语义列为
  **UX3.0 先行验证项**（小成本 spike，不通过则启用备选方案 B）。
- 备选方案 B（仅当 3.0 失败）：`--launch` 时 spawn 自身无窗口子进程，父进程
  保留 console 子系统；语义等价但多一层进程。

### 3.2 日志与错误可见性（无黑框 ≠ 黑盒）

- 新增 logger 模块：`data/logs/server-YYYYMMDD.log` 按天滚动，保留 14 天；
  **增量迁移**——main/worker/关键错误路径先接，存量 println 分批收编，不阻塞本轮。
- panic hook 同文件落盘。
- 致命启动错误（端口被非本服务占用/资产缺失/DB 打不开）：GUI 模式弹原生
  MessageBox（用户没有控制台可看——沿用便携启动链排障的教训）。
- 设置 → 服务控制新增「日志尾部」查看（复用插件日志 tail 先例）。

### 3.3 启动件切换

- 安装器快捷方式与 postinstall 改指 `labsentinel-server.exe --launch --tray`。
- `LabSentinel.bat` 保留一个版本周期作兼容（内容改为 `start "" exe --launch --tray`，
  不留窗口），下版本退役。

### 验收

- [ ] 双击 exe：无控制台窗口、托盘出现、浏览器打开
- [ ] `--console` 可看输出；日志文件滚动正确；panic 落盘
- [ ] CI 冒烟/smoke_rust/安装器 selftest 全绿（UX3.0 实证）
- [ ] 致命错误弹窗可见且日志留痕

---

## UX4 系统托盘

### 4.1 技术选型

- `tray-icon` crate（tauri 团队维护、纯 Rust、win32）+ 独立消息循环线程；
  图标内嵌 `include_bytes`（`web/icons/icon-192.png` 转 .ico 复用品牌图）。
- 不引入 Tauri/Electron/winit 整套运行时（保持单二进制与体积纪律，预估增量 <1MB）。
- 依赖进 cargo-deny 门禁（MIT/Apache-2.0 预期无碍，过门为准）。

### 4.2 交互

```
（双击托盘 = 打开控制台）
┌ LabSentinel 0.2.x · 127.0.0.1:8000 · 检测中 ┐  ← 禁用状态行/tooltip
│ 打开控制台                                    │
│ 复制访问地址                                  │
│ ──────────────────────                       │
│ 退出 LabSentinel                              │  ← 优雅关停
└──────────────────────────────────────────────┘
```

- **退出 = 优雅关停**：进程内直接调 `shutdown_server` 同一函数（与网页
  「停止服务」同语义；不走 HTTP 自环）——停 workers → flush WAL → 关 listener → exit。
- 「暂停检测/恢复」（P2 可选）：托盘一键挂起帧循环分析但不退服务。
- 告警角标（P2 可选）：最近 5 分钟有未处理事件时图标变色。
- 降级：非 Windows / 托盘创建失败（无 shell 会话）→ 日志 warn，服务照常运行。

### 验收

- [ ] 托盘出现、菜单各项可用；双击开控制台
- [ ] 退出后进程结束、端口释放、无残留（对齐安装器零残留口径）
- [ ] 托盘失败降级不崩服务

---

## UX5 安装器 / 冒烟 / 文档适配（收尾）

- `installer.iss`：快捷方式目标改 exe（`--launch --tray`）；`[Run]` postinstall
  同步；卸载 `taskkill` 不变（进程名未变）。
- `smoke_rust.py`：增 GUI 形态断言（就绪/版本/前端；托盘交互 CI 无法真点，
  以「无窗口启动不报错 + shutdown API 可用」为口径）。
- 文档三处：安装与卸载 / README 快速开始 / 部署指南（无黑框、托盘、三模式、
  bat 兼容期说明）；CHANGELOG。
- 版本节奏：合并进 0.3.0（SemVer minor——新增能力 + UI 重组）。

---

## 排期与依赖

```
UX3.0 预检（GUI 子系统管道语义 + tray-icon 过 deny）   ←小时级，先行
   │
   ├─ UX2 后端（detect.* API + cascade 引擎 + 验收扩展）   ←最大块
   ├─ UX1 前端信息架构（可与 UX2 并行）
   │        └─ UX2 前端（模式卡片三选一）
   ├─ UX3 实现（windows_subsystem + 日志 + MessageBox）
   └─ UX4 托盘
            └─ UX5 收尾（安装器/冒烟/文档/CHANGELOG）
```

## 风险与对策

| 风险 | 对策 |
|---|---|
| GUI 子系统吞 stdout，CI/安装器自检失效 | UX3.0 先行验证管道继承；备选方案 B（父子进程分离） |
| 模式切换不热生效 | PUT analyzer 现有行为核对；worker 软重启对齐 RA1.2 生命周期；验收覆盖 |
| API key 泄漏面扩大 | fernet `enc:v1:` + GET 脱敏 + 日志禁打；复用告警渠道凭据先例 |
| 误外发截图（合规） | allow_remote 双确认（DB 或 env 任一为真才放行远程），未确认拒绝并落 error 事件 |
| 预算计数重启清零 | 计数持久化 app_settings（detect.usage.*） |
| tray-icon 引入体积/许可问题 | cargo-deny 门禁已有；不通过则换 `systray`/自写 win32（工作量备选） |
| bat 退役破坏老用户习惯 | 保留一个版本周期 + 文档标注；安装器已统一走 exe |
| 导航合并引起旧入口失效 | 旧路由 replace 重定向；外链兼容 |

## 非目标

- 不做 UI 多语言（安装器双语保留）；不做移动端/响应式重设计。
- 不改系统服务（WinSW）模式；不做 macOS/Linux 托盘（Windows 优先路线）。
- 不重构后端既有 API 契约（仅新增 detect.* 三条 + analyzer 取值扩展）。
- 不做全量 println → 日志框架的一次性迁移（增量收编，另立长期项）。
