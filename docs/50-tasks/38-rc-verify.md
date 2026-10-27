# 任务 38 · v0.1.0-alpha 发布候选复核（文档 + 验证，不改业务代码）

> 背景：M1 功能与 v2 模型已就绪，发布唯一硬阻塞是「真实 RTSP 摄像头 24h 稳定性验证」（用户侧硬件）。在等待期间先把发布候选（RC）能自动验证的部分全部跑一遍，形成可引用的证据。

## 目标

1. **跑全套验证**（按顺序，单任务串行）：
   ```bat
   cd /d D:\User\Desktop\TOOLS\监控识别\server
   D:\LabSentinel\m0-venv\Scripts\python.exe -m pytest tests -q
   cd ..\web
   npm run build
   cd ..
   D:\LabSentinel\m0-venv\Scripts\python.exe tools\check_model_ready.py
   ```
   记录：pytest 通过数、build 产物体积、check_model_ready 的 PASS/FAIL/WARN。
2. **静态核对**：
   - `git status --short`（应干净；`dist/`、`server/data/`、`.env`、模型、venv 均为忽略项）；
   - 仓库内 `TODO/FIXME/XXX` 零命中（排除 node_modules/dist/docs/50-tasks 历史报告）；
   - 源码/文档无密钥（`rg` 或 grep Roboflow key/SMTP 密码/`LS_ADMIN_TOKEN=` 赋值）；
   - `.github/workflows/*.yml` YAML 可解析（`python -c "import yaml..."` 或 ruby/py 工具，至少括号/缩进目检）；
   - `LICENSE`/`NOTICE`/`server/.env.example` 存在且为最新。
3. **更新 `docs/发布核对报告.md`**：新增「2026-10-28 RC 复核」小节，写本轮证据与结论；明确：
   - 可自动验证项：全绿/异常清单；
   - 唯一剩余阻塞：真实摄像头 24h（用户侧）；
   - 不发布任何未验证数字，不写"已认证"等措辞。
4. **不改业务代码/不启停服务/不改模型**；若发现必须修的代码问题，写进报告"建议任务"，不要顺手改。

## 验收标准

- pytest 全绿、npm build 通过、check_model_ready PASS（WARN 可接受，需列出）；
- 报告新增小节含命令、数字、结论；
- `git status` 干净（除本任务文档/报告修改）；
- 报告明确"发布仍被真实摄像头验证阻塞"。

## 执行报告

（待填：pytest/build/模型自检结果 / 静态核对结果 / 报告更新位置 / 建议任务）

### 执行报告（flash，2026-10-28 01:05 交付）

- **全套验证（串行实跑）**：pytest **78 passed**（2 warnings 为死信测试预期日志，12.2s）；
  npm build 通过（486ms，gzip 42.77KB）；check_model_ready **PASS（FAIL 0 / WARN 1 / 11 项）**，
  唯一 WARN=`LS_PPE_VIOLATION_CLASS` 未显式设置（服务端默认值，不阻塞）。
- **静态核对**：git status 仅含任务 35–38 交付物（dist/data/.env/模型/venv 均忽略）✓；
  TODO/FIXME/XXX 源码 0 命中 ✓；密钥扫描无泄露（唯一命中为 .env.example 注释占位符）✓；
  workflows YAML safe_load 全过 ✓；LICENSE/NOTICE/.env.example 存在且最新 ✓。
- **报告更新**：docs/发布核对报告.md 追加「2026-10-28 RC 复核」小节（全套验证表/静态核对表/
  结论：自动可验证项全绿；**发布仍被真实摄像头 24h 验证阻塞**；建议任务 1 条）。
- **建议任务**：`.env.example` 补 `LS_PPE_VIOLATION_CLASS` 显式示例行（消除唯一 WARN，一行改动，
  留主线/下一任务处理——本任务约束不改业务代码故未顺手改）。
- **遗留问题**：无。任务 38 = 队列清空，35–38 全部交付。
