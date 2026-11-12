# LabSentinel 文档索引

> 文档按用途分组。入口：仓库根 [README](../README.md)。

## guide/ — 使用与部署（面向管理员/运维）

| 文档 | 内容 |
|---|---|
| [部署指南.md](guide/部署指南.md) | Windows 部署、摄像头接入、告警配置、合规自查 |
| [部署指南-Linux.md](guide/部署指南-Linux.md) | Linux systemd 原生部署（Docker 路线已终止） |
| [安装与卸载.md](guide/安装与卸载.md) | 安装包（setup.exe）与便携包两种形态：安装/升级/卸载/自启/杀软告警 |
| [运维手册.md](guide/运维手册.md) | 日常运维一本通：巡检/排障/备份恢复/升级回滚/故障速查 |
| [摄像头接入Runbook.md](guide/摄像头接入Runbook.md) | 摄像头接入一条龙：预检/接线/区域规则/告警验证/24h 观察 |
| [摄像头24h验收清单.md](guide/摄像头24h验收清单.md) | 24h 稳定性验收执行表 |
| [图片文件夹接入.md](guide/图片文件夹接入.md) | 截图目录接入：关键/违规定义、配置与排障 |
| [部署FAQ.md](guide/部署FAQ.md) | 踩坑集中答疑；启动失败先跑 `python scripts/doctor.py` |
| [杀毒软件误报说明.md](guide/杀毒软件误报说明.md) | 杀软启发式误报的用户处理指引 |
| [快速开始.md](guide/快速开始.md) | demo 包/源码一键入口 |

## dev/ — 开发者

| 文档 | 内容 |
|---|---|
| [架构设计.md](dev/架构设计.md) | 双运行时架构：Python 主运行时 + Rust 数据面/控制面、模块地图、线程模型、约束 |
| [api-reference.md](dev/api-reference.md) + [openapi.json](openapi.json) | REST/WS API 完整契约（客户端开发唯一依据） |
| [mqtt-protocol.md](dev/mqtt-protocol.md) | MQTT 事件总线协议 v1 |
| [plugin-guide.md](dev/plugin-guide.md) | 插件（外挂）指南：清单/权限/示例/安全红线 |
| [model-onboarding.md](dev/model-onboarding.md) | 模型接入手册（换模型零改代码） |
| [data-flywheel.md](dev/data-flywheel.md) | 数据飞轮：采集→预标注→训练→部署闭环 |
| [requirements.md](dev/requirements.md) | 需求文档 v0.2 |
| [GLOSSARY.md](dev/GLOSSARY.md) | 领域术语表 |
| [../adr/](adr/) | 架构决策记录（0001–0010） |

## releases/ — 发布

| 文档 | 内容 |
|---|---|
| [release-notes-v0.2.0.md](releases/release-notes-v0.2.0.md) | **0.2.0 发布说明**（Highlights/行为变化/已知问题） |
| [risk-register.md](releases/risk-register.md) | 风险登记册（A1 DirectML 并发死机复盘等） |
| [perf-baseline.md](releases/perf-baseline.md) | 推理性能基线（CPU/DML/Rust 各后端实测） |

## plan/ — 项目管理（活跃）

| 文档 | 内容 |
|---|---|
| [status.md](plan/status.md) | **任务状态台账**（R2 轮持续更新，附实测证据） |
| [78-review-round2-plan-2026-11-07.md](plan/78-review-round2-plan-2026-11-07.md) | 二轮审查与推进计划（归档正本） |

## archive/ — 历史留存（只读）

[00-vision.md](archive/00-vision.md)（愿景 v0.2）· [40-roadmap.md](archive/40-roadmap.md)（M0 手册）·
[70-release-plan.md](archive/70-release-plan.md)（v0.1 发布规划）· [58-packaging-strategy.md](archive/58-packaging-strategy.md)（打包方案对比）·
[验证记录-S7-L1黄金语料对齐.md](archive/验证记录-S7-L1黄金语料对齐.md)（S7 验收留档）·
[release-notes-v0.1.0-alpha.md](archive/release-notes-v0.1.0-alpha.md) 与 [发布公告.md](archive/发布公告.md)（v0.1 草稿）·
[20-research/](archive/20-research/)（预研）· [opentip-申诉材料-0.2.0.md](archive/opentip-申诉材料-0.2.0.md)（杀软申诉预备材料，勿主动提交）

## assets/

[README.md](assets/README.md)（界面截图，含 [mobile/](assets/mobile/README.md) 移动端基线）。
