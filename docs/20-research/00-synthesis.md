# 20 · 预研综合报告（第一轮）

> 调研日期：2026-10-25 · 方法：联网检索一手来源（官方仓库/官方文档/法规原文）
> 深度：选型级（够做决策）；标注 ⚠️ 的为"未完全验证"，进入开发前需复核

## TL;DR 推荐技术路线（草案，待用户拍板）

1. **架构**：自研 Python 服务端（FastAPI）+ 组装 CV 库，不基于 Frigate 二开；用 go2rtc/MediaMTX 做流媒体网关（WebRTC 低延迟看流）。
2. **检测模型**：若项目选 **AGPL-3.0** → Ultralytics YOLO（生态最好）；若选 **Apache-2.0** → RF-DETR 或 YOLOX。
3. **跟踪计数**：ByteTrack + 区域/越线判定逻辑（或 Roboflow supervision，⚠️ 协议待验证）。
4. **人脸比对**：集成 CompreFace（Apache-2.0，自带 REST API 与 1:N）或 SeetaFace2（BSD-2）；**不用 InsightFace 预训练模型**（协议不允许再分发）。
5. **客户端**：MVP 用 Web（建议 Vue3）；M3 用 uni-app x 一套代码出 安卓/iOS/鸿蒙/小程序。
6. **数据集**：Roboflow Universe 的 labcoat / LAB PPE 小数据集起步，自建数据集微调，中期反哺开源社区。

---

## 1. 底座：基于现成平台二开 vs 自研

| 候选 | 协议 | 定位 | 结论 |
|---|---|---|---|
| [Frigate](https://github.com/blakeblackshear/frigate) | MIT | 面向 Home Assistant 的家用 NVR，支持自定义 ONNX/OpenVINO YOLO 模型 | **参考其架构，不作为底座** |
| [go2rtc](https://github.com/AlexxIT/go2rtc) / MediaMTX | MIT（⚠️ 复核） | 流媒体网关：RTSP → WebRTC/HLS 给各端播放 | **采用**（Frigate 同款思路） |
| Shinobi / ZoneMinder / AgentDVR | 多为 GPL 系 | 传统 NVR，AI 能力弱或封闭 | 排除 |
| [CodeProject.AI](https://github.com/codeproject/CodeProject.AI) / Double Take | 有开源部分 | 通用推理服务 / 人脸识别配套 | 备选参考 |

**推荐自研服务端的理由**：① 需求是"实验室领域语义"（安全区×防护合规×时段规则），与家用 NVR 的"人/车检测+录像"差异大；② 目标包含学习价值（用户诉求），组装级自研正好；③ 产品化中文 UI/告警流程需要完全自主的前后端；④ Frigate 与 Home Assistant 深度耦合。**借鉴点**：低分辨率连续检测+事件触发高分辨率复核、事件驱动录像、go2rtc 统一取流。

## 2. 检测模型：协议是第一约束

| 方案 | 协议 | 特点 |
|---|---|---|
| Ultralytics YOLOv8/11 | **AGPL-3.0（含模型权重）** | 生态最强：训练/导出/文档/数据集格式/ByteTrack 内置 |
| [RF-DETR](https://github.com/roboflow/rf-detr)（Roboflow） | Apache-2.0 | 实时 DETR，2025-2026 精度超 YOLO11 同级，Roboflow 主推 |
| D-FINE | Apache-2.0 | 实时 DETR，RF-DETR 同期作品 |
| YOLOX | Apache-2.0 | 经典可商用 YOLO 系，生态略老 |

关键事实：Ultralytics 于 2023-04 从 GPL-3.0 改为 **AGPL-3.0，且权重同样受协议约束**——闭源商用需买企业授权；开源项目使用则**整个项目必须 AGPL-3.0**（AGPL 单向传染，不能降级到 Apache）。来源：[Ultralytics License](https://www.ultralytics.com/license)、[Roboflow 的 AGPL 风险分析](https://roboflow.com/blog/why-agpl-3-0-is-a-risk-for-computer-vision-teams)。

**联动决策（ADR-0001 候选）**：
- **A. 项目 AGPL-3.0 + Ultralytics**（推荐）：单人开源项目，AGPL 义务几乎无感；换来最全生态（自训实验服模型最省力）。缺点：未来若有人想闭源商用你的代码会受限（对开源项目反而是保护）。
- **B. 项目 Apache-2.0 + RF-DETR/YOLOX**：协议最宽松，检测生态换一家，训练管线要自己拼（RF-DETR 也提供训练支持，Roboflow 生态可用）。

## 3. 数据集（实验服/PPE）

Roboflow Universe 已有小规模现成数据集，可直接导出 YOLO 格式：

- [Labcoat 数据集集合](https://universe.roboflow.com/search?q=class%3Alabcoat)（GSFC University 的 Lab Coat Detection：229 图）
- [LAB PPE（Gavriels Workspace）](https://universe.roboflow.com/gavriels-workspace-e9azv/lab-ppe-p4qe4)：含 Gloves/Goggles/Lab Coat/Mask/Person 类别
- [Safety_PPE](https://universe.roboflow.com/safety-jmser/safety_ppe)：实验室场景，护目镜+实验服

结论：**冷启动可用，规模偏小（几百张级）**。计划：合并现有集 + 自行拍摄标注（自己实验室+公开视频截帧）→ 微调；中期把扩充后的"实验室 PPE 数据集"作为开源贡献发布（项目差异化亮点）。注意 Roboflow Universe 各数据集协议不一（多为 CC 系），**合并发布前逐个核对**。

## 4. 人脸比对：协议陷阱已排雷

| 方案 | 协议 | 形态 | 结论 |
|---|---|---|---|
| InsightFace | 代码 MIT，**模型仅非商业研究用途** | 库 | ❌ 模型不可随开源项目分发（[官方声明](https://github.com/deepinsight/insightface)、[issue #2022](https://github.com/deepinsight/insightface/issues/2022)） |
| [CompreFace](https://github.com/exadel-inc/CompreFace) | Apache-2.0 | 独立 REST 服务，自带 1:1/1:N 与管理界面 | ✅ 首选（集成成本低；⚠️ 2026 活跃度待复核） |
| [SeetaFace2](https://github.com/seetafaceengine/SeetaFace2) | BSD-2-Clause | C++ 工具包 | ✅ 备选（嵌入自研、无外部服务） |

**推荐**：MVP 不做人脸（匿名计数模式）；M2 以 CompreFace 独立服务集成（Docker Compose 加一个容器，协议干净，省去自建特征管理）；若追求纯嵌入式再用 SeetaFace2。

## 5. 跟踪与计数

- **ByteTrack**：当前多目标跟踪主流选择，Ultralytics 内置 track 接口；选路线 B 时也有独立 Python 实现（MIT，⚠️ 复核具体实现协议）。
- **[supervision](https://github.com/roboflow/supervision)**（Roboflow）：区域入侵/越线计数 API 开箱即用；协议需复核（印象为 MIT，**未验证**）。
- 区域判定逻辑简单（目标框中心/底边点与多边形求交），自研成本低，不强依赖库。

## 6. 跨平台客户端（五端覆盖）

| 方案 | 安卓/iOS | 鸿蒙 NEXT | 小程序 | 评估 |
|---|---|---|---|---|
| **uni-app x**（DCloud） | ✅ | ✅ 4.61+ 编译为 ArkTS 原生（2025-04 起） | ✅ 微信 | **唯一一套代码全覆盖五端**；需 HBuilderX；编译器非完全开源（⚠️ 开源项目需注明） |
| Flutter（OHOS 移植） | ✅ | ⚠️ 社区/华为移植，SDK 跟进至 3.22.1，非官方 | ❌ | 备选 |
| React Native（ohos） | ✅ | ⚠️ OpenHarmony SIG 维护，0.77.x | ❌ | 备选 |
| 纯 Web/PWA | 浏览器 | 浏览器 | ❌ | MVP 用；手机浏览器可用响应式 Web 顶一阵 |

来源：[uni-app x 鸿蒙开发指南](https://doc.dcloud.net.cn/uni-app-x/app-harmony)（2026-02 更新）、[uni-app 鸿蒙概述](https://en.uniapp.dcloud.io/tutorial/mp-harmony/intro.html)、[OpenHarmony-SIG ohos_react_native](https://gitee.com/OpenHarmony-SIG/ohos_react_native)、[OpenHarmony flutter_flutter](https://gitcode.com/openharmony-tpc/flutter_flutter)。

**建议**：MVP = 响应式 Web（Vue3，与 uni-app 技术栈同源，后期迁移顺滑）；M3 = uni-app x 出四端。鸿蒙不单独原生开发（ArkTS），走 uni-app x 编译。

> ⚠️ **决策更新（2026-10-25，晚于本节调研）**：用户拍板客户端拒绝 WebView 套壳与闭源工具链，本节的 uni-app x 建议被推翻，改走 Flutter 方向（Android → Win/mac/Linux 桌面 + iOS），小程序降级为可选独立实现。详见 [adr/0003](../adr/0003-native-client-flutter.md)。本节保留作为调研记录。

## 7. 合规硬约束（人脸识别，中国境内）

依据：[《人脸识别技术应用安全管理办法》](https://www.cac.gov.cn/2025-03/21/c_1744174262156096.htm)（网信办+公安部，2025-03-21 公布，**2025-06-01 施行**）+ PIPL：

1. **单独同意**：处理人脸信息须取得个人单独同意（不能混在通用条款里）。
2. **不得作为唯一验证方式**：存在替代方式时不得将人脸作为唯一验证/识别手段。
3. **公共场所安装**：须为维护公共安全所必需、合理确定采集区域、**设置显著提示标识**。
4. **备案**：存储人脸信息达 **10 万人**起 30 个工作日内向省级网信部门备案（实验室规模远达不到，但要在文档写明阈值）。
5. **个人信息保护影响评估（PIA）**：处理敏感个人信息前需评估。

→ **转化为产品设计约束**（已写入 00-vision 与 10-requirements）：默认匿名计数；人脸模块默认关+开启需确认+内建同意登记；部署文档含"现场提示标识"模板与合规自查清单；人脸数据本地存储+保留期+一键清除。

## 8. 竞品格局

- 商业方案已较成熟：[诺飞尔智慧实验室](https://www.nofar.com.cn)（宣称准确率 >98%）、[华安泰 AI 行为分析](https://www.hatcctv.com/AIxingweifenxiyujingxitong.html)（25+ 行为/15 物品）、高校园区级方案（[insytone](https://insytone.com/node/6884) 等），以及海康/大华的通用园区 AI。
- 共同点：闭源、整体方案销售、面向园区级客户。**没有发现面向中小实验室的开源自部署方案**——即本项目的空位。
- [Ultralytics 官方博客](https://www.ultralytics.com/zh/blog/computer-vision-for-smarter-lab-workflows)也把实验室 PPE 监控列为 CV 应用场景（侧面验证需求真实性）。

## 9. BetterGI 参考价值评估（回应立项时的疑问）

[BetterGI](https://github.com/babalae/better-genshin-impact)（GPL-3.0，.NET）= 屏幕捕获 + YOLO 目标检测 + PaddleOCR/Yap 文字识别 + 模拟输入的游戏自动化工具。**与本项目的相似只在检测模型层**（都用 YOLO 系实时检测），架构完全不同（它处理屏幕帧并操控输入，我们处理 RTSP 流并产生事件）。可借鉴：① 证明 YOLO 在消费级硬件实时检测的工程可行性；② 它"模型文件随仓库分发+社区共建训练数据"的运营方式值得学。

## 未验证清单（进入 M0 前复核）

- [ ] supervision 库协议（印象 MIT）
- [ ] go2rtc / MediaMTX 协议与 WebRTC 在弱网下的表现
- [ ] CompreFace 2026 年活跃度与模型协议细节
- [ ] 纯 CPU（如 i5）跑 1 路 1080p 检测的实测帧率（M0 实测项）
- [ ] uni-app x 对视频播放（WebRTC/HLS）与推送通知的支持成熟度
- [ ] ByteTrack 独立实现的协议
