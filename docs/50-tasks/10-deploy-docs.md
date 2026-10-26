# 任务 10 · 部署文档与配置模板完善（先读本目录 README.md）

## 背景

README.md 的部署章节还是骨架。要给真实部署者（实验室管理员）一份"照着做就能跑"的文档。

## 目标

1. **`server/.env.example`**：把 config.py 里全部 LS_* 配置整理成带中文注释的模板（含 6 个告警渠道 + 人脸预留 + 模型路径 + 阈值组），与 config.py 逐项对齐（读源码核对默认值，别抄 README）
2. **`docs/部署指南.md`**：
   - Windows 原生部署（步骤复用 README 快速开始但更细：Python 版本、venv、模型文件从哪来——引用 docs/40-roadmap.md 与任务 06 工具链）
   - Docker/Linux 部署（docker-compose.yml 已有；补：models 目录挂载、数据持久化、日志查看）
   - 摄像头接入章节：海康/大华 RTSP 地址格式模板（`rtsp://user:pass@ip:554/Streaming/Channels/102` 子码流示例）、手机"IP 摄像头"App 测试法
   - 告警渠道配置分节：每个渠道一张"去哪拿 key/webhook"说明表（企微群机器人/钉钉机器人/飞书机器人/Server酱/PushPlus/Qmsg/邮箱 SMTP 授权码）
   - 合规自查清单（引用 20-research/00-synthesis.md §7，逐条可勾选）
3. **README.md** 瘦身：快速开始保留，详细内容链接到部署指南
4. `docs/50-tasks/README.md` 任务清单表补上 09/10 两行

## 验收标准

- .env.example 与 config.py 字段一一对应（人工核对，写进执行报告）
- 部署指南可让"会装 Python 但不懂本项目"的人零上下文完成部署
- 不改动任何 Python/JS 代码

## 执行报告

### 交付物
1. **`server/.env.example` 全量重写**：与 config.py 逐项对齐（模型路径 / 推理 6 项 / PPE 5 项 / go2rtc 2 项 /
   告警渠道 12 项 / 人脸 5 项），默认值照抄源码，每项中文注释。核对结论：config.py 全部字段均有对应注释行，
   无遗漏、无多余项。
2. **`docs/部署指南.md`（新建）**：部署形态选型表、Windows 原生（venv/依赖铁律/一键启动/生产化/7 条常见问题）、
   Docker/Linux（数据卷/日志/人脸 profile）、摄像头接入（海康/大华/宇视 RTSP 模板、手机模拟、编码注意）、
   告警渠道申请表（8 渠道"去哪拿 key"）、人脸合规 8 项自查清单、部署后 5 分钟验收自检。
3. **README 瘦身**：快速开始后加部署指南链接；Docker 段保留最小命令并链接到指南对应章节。
4. `docs/50-tasks/README.md` 任务清单表 09/10/11 行已由用户并行补上（未重复修改）。

### 验收结果
- `.env.example` 与 config.py 字段对照：人工逐项核对通过（40 个字段全部覆盖）
- 未改动任何 Python/JS 代码；pytest 37 passed 不受影响
- 部署指南中的命令/路径均取自仓库真实结构（venv 路径、start-all.bat、compose profile、API 路径）

### 遗留问题
- 指南中 CompreFace compose 段标注为 best-effort（本机无 Docker 未实测）
- nginx/IIS 反代与开机自启仅给方案，未写具体配置样例（按部署环境差异大，留待现场适配）
