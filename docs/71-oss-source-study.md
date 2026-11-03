# OSS 源码级参考点研究（给主代理，2026-11-03 flash）

> 用户指示：不止 README 对标——**实际拉取源码阅读**，找出可参考的实现点并标注出处，
> 结合本项目给出修改建议。方法：8 个仓库浅克隆至 `D:\tmp\oss-study\`（frigate/frigate-notify/
> label-studio-ml-backend/pywebview/go2rtc/scrypted/localsend/double-take），深读关键文件；
> 本项目侧仅提出落点，**未改任何代码**。69 号文档是概览篇，本文是源码证据篇，建议编号沿用 69。

## 一、Frigate（blakeblackshear/frigate，★37184）——四个可直接照抄的契约

### 1.1 本地 VLM 二次判读（建议#12 的完整蓝本）⭐最高价值
- **出处**：`frigate/genai/__init__.py`（`register_genai_provider` 插件注册器 + `PROVIDERS` 表）、
  `frigate/genai/plugins/ollama.py`（Ollama 异步客户端、图片 base64 交错注入、token/耗时统计提取）、
  `frigate/genai/prompts.py`（结构化输出 schema + 逐字段引导词，注释明确说明"grammar-constrained
  providers 会按 cap 截断，所以每个字段都要写长度要求"）、`frigate/config/camera/genai.py`（per-camera 配置）。
- **机制**：事件落库后把 clips/截图发给配置的 GenAI provider，生成结构化描述（observations/scene/title/
  threat level）写回 Event。
- **LabSentinel 落点**：新建 `server/app/genai.py`（照抄注册器+prompts 结构）；挂钩点在
  `pipeline.py` 事件落库处或 `alerts.py` dispatch 前（异步，不阻塞主链路）；配置项
  `LS_GENAI_PROVIDER/LS_GENAI_MODEL/LS_GENAI_PROMPT`；输出两个用途——①事件 detail 增强（一句话描述）；
  ②**误报预分类**（"画面中无实验室人员/无明显违规 → probable_false 标记"），结果只写 events 扩展列
  或 detail 前缀，不自动改 status（人工确认兜底）。
- **关键工程细节（Frigate 用血泪换的）**：响应必须 JSON schema 约束+逐字段长度引导；图片用
  `to_jpeg` 压缩后注入；失败静默降级+统计 token 数（`_extract_ollama_stats`）；provider 失败不影响事件主流程。

### 1.2 检测器抽象契约（未来多后端的地基）
- **出处**：`frigate/detectors/detection_api.py`（`DetectionApi` 抽象类：`type_key` + `detect_raw(tensor_input)`
  + `ensure_dependencies()` 运行时依赖清单）；`frigate/detectors/plugins/`（cpu_tfl/edgetpu/openvino/onnx/
  hailo/axengine…12 种后端同契约）。
- **LabSentinel 落点**：现在 `engine.py` 直连 DirectML 单后端——不必立即抽象；但**新增任何推理能力**
  （姿态/异常/开放词汇）时应按"`detect_raw` 级最小接口 + 独立 worker"接入（对应 69 建议#14/#19 的
  "检测微服务外挂位"），避免引擎文件继续膨胀。

### 1.3 事件前缓冲（建议#17）
- **出处**：`frigate/config/camera/record.py`（`pre_capture` 字段 + `event_pre_capture()` 合成逻辑——
  alerts/detections 各自配，取最大值）。
- **LabSentinel 落点**：我们无持续录像，但 `pipeline.py` 的 `CameraWorker` 天然持有逐帧——事件触发时
  回填**事件前 N 秒的内存帧环形缓冲**（如 5s@10fps=50 帧 JPEG），用 ffmpeg 合成 mp4 存入 videos/；
  go2rtc 侧 `internal/mp4`（自带 MP4 mux）可作为 RTSP 路的替代实现。工作量 1-2d，价值=证据完整性
  （事件前画面往往是判定关键）。

### 1.4 MQTT 主题与负载规范（建议#11 的反向设计）
- **出处**：`frigate/comms/mqtt.py`（`publish()` 统一 `{topic_prefix}/{topic}` 前缀 + retain 队列 + available 主题）；
  `frigate/comms/events_updater.py`（事件变更即发）。
- **LabSentinel 落点**：`alerts.py` 或新 `server/app/mqtt_bridge.py`（依赖 paho-mqtt，纯 Python 无 C 依赖，
  便携包体积 +~200KB）。主题/负载规范（**以消费者视角反向设计**，见下节 frigate-notify）：
  `labsentinel/events/{camera_id}/{type}`（事件新增）与 `labsentinel/alerts/{channel}`（告警投递结果）；
  payload 字段必须含：`id/ts/camera_id/camera_name/type/level/zones[]/detail/snapshot_url/api_base`，
  消费者即可零 API 依赖工作。配置 `LS_MQTT_HOST/PORT/USER/PASS/TOPIC_PREFIX`（默认关闭=无 MQTT 不报错）。

## 二、frigate-notify（0x2142/frigate-notify，★414）——"外挂消费者"的完整形态

- **出处**：`events/mqtt.go`（订阅 `{prefix}/{mode}`、自动重连、连接状态健康上报、3 次重试上限）、
  `events/filters.go`（消费侧过滤模型：`Cameras.Exclude` / `Zones.Allow|Block|Unzoned` / `MinScore` /
  `AboveMinScore`）、`events/cache.go`（事件去重缓存）。
- **可学点**：一个 2000 行的 Go 小项目就能围绕 Frigate 的事件协议建立完整产品（配置热加载/健康端点/
  多渠道通知/去重/测试按钮）。**这就是我们 MQTT+REST 协议开放后预期的第三方生态形态**——
  证明"协议即插件平台"的成本极低（建议#11 的核心论据）。
- **对我们的要求**：事件 MQTT payload 一旦发布就应视为**对外契约**（版本化 `v` 字段+只加不减）。

## 三、Label Studio ML Backend（heartexlabs/label-studio-ml-backend，建议#13 落点）

- **出处**：`label_studio_ml/model.py:195` ——`predict(self, tasks, context, **kwargs)` 即预标注契约：
  返回 `[{result: [...], score}]` 结构即自动填充标注界面；`:97 setup()` 启动时加载模型。
- **LabSentinel 落点**：写一个 ~150 行的 `ls_ml_backend`（包装我们的 ONNX 推理：输入=误报池图片，
  输出=11 类框预填）；误报池已有 `data/annotation_pool` 目录与《标注与模型迭代手册》——
  接上后流程变为"误报→自动预标注→人工只修正→导出训练"，**标注成本减半以上**。
  部署形态：`pip install label-studio-ml` + 本项目脚本，或纯文档指引（后者零依赖）。

## 四、pywebview（r0x0r/pywebview，建议#2 修订）⚠️含勘误

- **出处**：`webview/platforms/winforms.py`（`create_window`，.NET WinForms 宿主）+
  `webview/platforms/edgechromium.py`（WebView2 内核，Windows 默认）——多窗、JS 桥、全屏等齐备。
- **⚠️ 勘误（69 概览篇的假设需修正）**：实测 grep 全库**无内置托盘（tray/NotifyIcon）实现**——
  托盘需自行组合 `pystray`（Pillow 依赖）或用我们现成的 C# launcher 加托盘。
- **修订后方案**：LabSentinel 瘦壳 = pywebview(WebView2) 主窗 + `pystray` 托盘（运行状态色图标+
  菜单：打开面板/启停/退出）；或更省——C# `LabSentinel.exe`（launcher）本就是 GUI 子系统，
  直接在 launcher_exe 里加 NotifyIcon 托盘，**零新 Python 依赖**，且托盘菜单可直接调
  `--stop`/start 逻辑（推荐后者，工作量 1d 内）。

## 五、Scrypted（koush/scrypted，★6039）——插件 SDK 的能力接口范式

- **出处**：`sdk/src/index.ts`（`ScryptedDeviceBase` + `DeviceManager/EndpointManager/MediaManager`
  注入）、`sdk/src/acl.ts`（插件访问控制）、`plugins/` 官方插件仓（alexa/amcrest/coreml/…每个插件独立包）。
- **可学点**：插件不是"拿到全量 API"，而是**实现声明式能力接口**（DeviceProvider 等），宿主按接口
  注入最小依赖+ACL——这正是我们插件二期（69 建议#4）`capabilities` 枚举（read/network/write-events/
  camera-stream…）+ scoped token 的设计模板。一期先落"清单页+约定目录"，二期按此演进。

## 六、go2rtc（AlexxIT/go2rtc，★14564）——已在用的流底座补课

- **出处**：`internal/mp4`（内建 MP4 mux/导出）、`internal/api`（HTTP API 全量）、rtsp/restream 体系。
- **可学点**：①pre-event/pre-record 段导出可直接调 go2rtc API 实现（与 1.3 的帧缓冲方案二选一或并存）；
  ②我们的 `go2rtc.py` 热同步（PUT/DELETE streams）与它的 API 面一致，后续加"按需拉流录像"无需引入
  新组件——**强化"go2rtc 作为流媒体底座"的选型信心**（Frigate 同款）。

## 七、LocalSend / double-take / OBS（概览，出处见 69 号文档第五/六节）

- LocalSend：发布工程（winget/MSIX 全渠道）——建议#18；协议规范文档化值得抄（我们的 MQTT/REST
  协议一旦开放同步产出规范页）。
- double-take：多检测器统一 UI + MQTT 双向（`api/src/controllers/`）——"检测融合层"形态参考。
- OBS：插件 API 版本化与稳定性纪律——插件一期上线时照其 changelog/deprecation 流程。

## 八、给主代理的落地组合（按"最小可验收切片"重排）

| 切片 | 内容（建议#） | 为什么先做 | 预估 |
|---|---|---|---|
| **A 误报治理** | #12 VLM 预分类（Frigate genai 蓝本，含 prompts 工程细节）+ #13 预标注闭环 | **模型止损线是当前发布硬阻塞**；两件都不需要重训 YOLO | A1: 1-2d 验证；A2: 2-3d |
| **B 生态开放** | #11 MQTT 事件总线（payload 规范按 frigate-notify 消费模型反向设计）+ 插件一期 | 协议开放=最低成本拿到"插件生态"，社区可做 frigate-notify 式外挂 | 1-2d |
| **C 桌面形态** | #2 C# launcher 托盘（修订案，零新依赖）+ #1 View Transitions | 用户可感的最直接"产品化"提升 | 1-2d |
| 观望/后置 | #14/#15/#19（能力型检测扩展，走 1.2 的检测器插槽）；#16/#17（审查时间线/前缓冲）；#18 | 等切片 A/B 验证后再排 | — |

**风险与合规备注**：①VLM 本地推理与 DML 推理共享 GPU——必须走 `engine._run()` 同一串行锁或独立
CPU 量化模型（Frigate 用独立进程+队列，我们照做）；②MQTT 凭据走 .env（不进仓库/诊断包——61 的脱敏
清单需加 MQTT_PASS）；③人脸相关仍默认关闭（80-risk-register C3 不变）；④新依赖仅 paho-mqtt（B 切片）
与可选 ollama HTTP 调用（标准库 httpx 即可，零新增）。

> 出处标注方式：`仓库/文件路径:行号或函数名`，克隆快照 2026-11-03（shallow，各仓库 HEAD 见
> `D:\tmp\oss-study\`）。本文为研究产出，未改项目代码；采纳与否由主线定夺。

---

# 深度篇（2026-11-04）：第二批逐项目源码深读——每个项目都提炼专属可学点

> 应用户要求：不止概览——对每个有特点的项目落源码。sparse clone 控制体积
> （PowerToys 4MB/OBS 5MB/Immich 7MB/BetterGI 3MB），出处标到文件:行号/函数。

## 九、PowerToys·Awake 模块 —— "监控机防休眠"的现成实现 ⭐新建议 #20

- **出处**：`src/modules/awake/Awake/Core/Manager.cs:139-149`——核心就一个 Win32 调用
  `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ...)`，配合 AwakeStateCalculator
  管理"限时保持/无限保持/显示器也保持"三态。
- **LabSentinel 落点**：24h 挂机监控机最怕系统自动睡眠/硬盘休眠（告警链直接断）。三选一实现：
  ①launcher（C#，零依赖）加"防休眠"开关；②服务端托盘/设置页调 `powercfg`；
  ③Setup 篇文档写推荐电源计划。**推荐 ①**：`LabSentinel.exe` 启动时按 config.env
  `LS_KEEP_AWAKE=1` 常驻 `SetThreadExecutionState`，`--stop` 时恢复；UI 状态进 `/api/system`。
  工作量 0.5d，价值：真实 24h 验收的前置保障（这属于** ours 独有痛点**，Frigate 用户用
  HA/服务器不必管，Windows 挂机必须管）。

## 十、OBS Studio —— 插件"API 纪律"与前端扩展点

- **出处**（本地 sparse 实读）：`libobs/obs-module.h`——插件入口三件套
  `MODULE_EXPORT bool obs_module_load(void)` / `obs_module_ver()` / `obs_module_set_pointer()`
  （行 23-101）：**版本函数+加载函数+指针回填**构成可审计的最小插件契约。
- **前端扩展点**：OBS 前端插件可往主窗注入自定义 Dock 面板（`obs_frontend_add_dock`，
  UI/obs-frontend-api.h——raw 拉取校验受限，以官方文档为准）。
- **LabSentinel 落点**：①插件一期对外承诺就学这套"最小契约"：插件包=manifest.json+入口，
  宿主只保证 `load/info/unload` 三函数语义+版本号（不做 OBS 级 C ABI，做 JSON 契约）；
  ②**前端 Dock 启发**：我们 Vue SPA 可以做"插件面板注册"——插件清单里声明
  `ui.panel: {route, icon}`，前端动态挂一个"插件"导航区（源码不动，配置驱动），P3。

## 十一、LocalSend —— 协议即产品（v2.2 版本化协议）

- **出处**：`localsend/packages/core/src/model/discovery.rs`——`PROTOCOL_VERSION_V2 = "2.2"`
  常量、`DeviceType {Mobile, Desktop, Web, Headless, Server}` 枚举、`ProtocolType{Http,Https}`
  线上小写 serde 双向校验（专属 mod 处理大小写）；发现/传输跑在独立 isolate
  （`localsend_isolates/lib/src/task/discovery/`）。
- **可学点**：**协议三要素**——显式版本常量（不兼容即拒连）、设备类型枚举（Headless/Server 是一等公民）、
  严格的线上格式校验。我们的 MQTT/REST 事件契约（建议#11）照此办：payload 顶层加 `v` 字段、
  `device_type: "lab-monitor"`、字段 serde 式校验（pydantic `extra="forbid"`），并出一页协议规范。

## 十二、BetterGI —— 用户脚本体系的任务编排

- **出处**：`BetterGenshinImpact/Service/Interface/IScriptService.cs`——
  `RunMulti(GameRuntime runtime, IEnumerable<ScriptGroupProject>, groupName, CancellationToken)`：
  **脚本组**（配置组 JSON，`{scriptGroup.Name}.json`，ScriptService.cs:339）+ 运行时注入 + 取消令牌；
  脚本项目按清单顺序执行、通知模型含 ScriptNotificationData。
- **可学点**：面向用户的自动化不是裸脚本，而是**"脚本组清单+运行时+取消令牌+执行通知"四件套**。
  我们插件一期若支持"外挂任务"（如夜间自动导出报表、误报池自动预标注），照此设计：
  `plugin.json` 声明 tasks[]（入口/定时/依赖），宿主给 cancel token 与执行历史（audit_log 已有底子）。

## 十三、Immich —— ML 推理独立微服务的工程范式 ⭐印证检测微服务路线

- **出处**：`machine-learning/immich_ml/main.py:165`——整个 ML 服务就一个
  `POST /predict`（pydantic **`STRICT = ConfigDict(extra="forbid")`** 严格 schema）按类型分派；
  `machine-learning/immich_ml/pipeline.py`——每模型一个 frozen dataclass `Slot[O: Options]`
  （CLIP 文/图、人脸检测/识别、OCR 检测/识别各一槽），服务端经
  `server/src/services/smart-info.service.ts` 调用；任务编排走 server 侧 job 队列（job.service）。
- **可学点**：①**推理=独立 FastAPI 进程+单端点分派+严格 schema**——与主服务解耦，模型热插拔；
  这正是给 LabSentinel 未来多模型（YOLO+VLM+姿态）的部署形态模板（建议#12/#14 的落地形态）：
  `server/app/` 保持主推理，第二模型走独立小进程+`/predict`，靠 `engine._run()` 外的独立串行锁排队；
  ②每模型一个 Slot 类+Options frozen dataclass——我们 engine 的配置散在 settings，可借鉴成
  per-model 配置对象（低优先）。

## 十四、double-take —— 多检测器"融合层"模式

- **出处**：`api/src/controllers/match.controller.js:12-42`——多个检测器结果
  （frigate/compreface/compcv…）统一 `format(matches)` 归并、按置信度取最优、结果签名
  （jwt）后挂到 storage 路由；MQTT 双向（`api/src/mqtt`）。
- **可学点**：**融合层独立于检测器存在**——当 LabSentinel 同时跑 YOLO+VLM（A 切片）时，
  "谁说了算"要有明确规则（VLM 只标注 probable_false 不覆盖 YOLO 事件 = 我们已定的融合规则；
  double-take 证明这个分层值得独立模块化）。它的 JWT 签名快照路由也可抄给证据包外链场景（P3）。

## 十五、Scrypted（补）—— 插件进程隔离实锤

- **出处**：`server/src/plugin/plugin-remote-worker.ts:223`——**每个插件跑在
  `child_process` 或 `worker_threads`**（二选一按插件声明），宿主经 RPC（plugin-host-api.ts）
  注入能力；`sdk/src/acl.ts` 管访问控制；插件崩溃不影响宿主（plugin-host.ts 重启策略）。
- **可学点**：与 71 号文档第五节互证并**升级**——插件二期的隔离粒度应该是"每插件独立进程"
  （不是共享 vm），Python 侧即 `subprocess.Popen(插件入口)` + stdin/stdout JSON-RPC 或
  本地 HTTP，宿主崩溃重启上限（SPlayer RESTART_MAX_ATTEMPTS 同款）。成本可控：插件本来
  就是独立语言进程，这一步在方案 A（外置进程）里天然成立。

## 十六、frigate review API —— 审查时间线的数据模型（建议#16 补充）

- **出处**：`frigate/api/review.py`——多个区间查询端点（summary/按天/按 severity/分页，
  行 64-708）+ `get_review_from_event(event_id)`（事件→review 反查）。
- **可学点**：时间线 UI 的后端先有**聚合端点**（`/api/review/summary` 按天+严重级分桶）再拉明细；
  我们 Events 页若做时间线（建议#16），先加 `/api/events/timeline?days=7&level=` 聚合端点
  （一条 SQL group by date,hour 即可），前端缩略图条复用 snapshot 缩略列。

## 十七、无架构启示的盘点项（明确记一笔，避免后来者重复调研）

- 7-Zip/Git/gh CLI/Pandoc/WSL 发行版/Everything（主程序闭源，SDK 仅文件索引 API）/
  FlClash（代理核心与监控域无关）/剪映/Typora（闭源）：工具属性，无可迁移架构点。

## 十八、深度篇结论：三切片建议维持，新增 #20，修订 #2

| 变更 | 内容 |
|---|---|
| **新增 #20（P1）** | 监控机防休眠：launcher `SetThreadExecutionState` 开关（PowerToys Awake 同源 API），24h 验收前置 |
| **修订 #2** | 托盘从 pywebview 方案改为 **C# launcher 内置 NotifyIcon**（pywebview 无托盘实锤；launcher 已是 GUI 进程零依赖） |
| **A 切片强化** | VLM 判读部署形态采用 Immich 式独立 `/predict` 微服务（进程隔离+strict schema），不塞主服务 |
| **B 切片强化** | MQTT payload 契约加 `v`/`device_type` 字段+pydantic extra=forbid（LocalSend 协议三要素） |
| **二期强化** | 插件=每插件独立进程（scrypted 实锤）而非共享解释器 |

> 出处均标 `仓库/文件:行` 或函数名；克隆快照 2026-11-03/04（`D:\tmp\oss-study\`，sparse）。
> 仍未产出源码级启示的：Immich 前端/OBS 前端插件面板细节（文档级已引用）、MediaPipe/anomalib
> （模型库，等能力扩展立项再读）、7-Zip 等工具项（见十七节）。
