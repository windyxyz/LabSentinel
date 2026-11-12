# 20 · 预研索引

| 报告 | 状态 | 主题 |
|---|---|---|
| [00-synthesis.md](./00-synthesis.md) | ✅ 第一轮（选型级） | 全部主题的综合对比与推荐（2026-10-25） |

## 第二轮深挖清单（按需启动，逐个出独立报告）

原计划的四个深挖方向（第一轮因模型并发限制由主会话直接完成选型级调研，深挖推迟）：

1. **01-opensource-video-platforms.md**：Frigate 架构细读（低分辨率检测+事件复核机制）、go2rtc vs MediaMTX 对比、Docker 部署形态。
2. **02-cv-models.md**：选定路线后的模型微调实验（Ultralytics 或 RF-DETR 训练管线跑通 labcoat 数据集）、supervision/ByteTrack 协议复核、CPU 帧率实测。
3. **03-cross-platform-clients.md**：uni-app x 视频/推送能力验证（到 M3 前再做）、uni-app 商业条款对开源项目的影响。
4. **04-market-and-compliance.md**：竞品功能清单细化（作为功能对标表）、合规自查清单成文（部署文档引用）。

## 调研方法约定

- 优先一手来源（官方仓库、官方文档、法规原文），每条关键结论附 URL。
- 查不到的标"未验证"，禁止编造；选定进入开发的依赖必须复核协议。
- 报告头部写调研日期，过期结论（尤其鸿蒙生态、模型版本类）使用前重查。
