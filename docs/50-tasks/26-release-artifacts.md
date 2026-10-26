# 任务 26 · 发布物准备（LICENSE/NOTICE/.gitignore/CI）——先读本目录 README.md

> 来源：docs/发布核对报告.md 发布阻塞项 1–6。目标：v0.1.0-alpha 的法律与 CI 项全绿。

## 目标

1. **LICENSE**：根目录放 AGPL-3.0 全文（官方原文，勿改一字）。网络不可达时用 `https://www.gnu.org/licenses/agpl-3.0.txt`（走 ghfast 不行——非 GitHub 域；可直连或从本地 pip 包内复制同文），文件头保留原文
2. **NOTICE**：第三方组件声明（go2rtc MIT、ffmpeg GPL、OpenH264 BSD、Ultralytics AGPL（模型未入库仅训练用）、Roboflow 数据集 CC BY 4.0 署名、supervision MIT），格式：组件/版本/许可/用途/来源链接
3. **.gitignore**（按核对报告 1.2 清单）：`server/.env`、`server/data/`、`m0-spike/*.onnx`、`m0-spike/*.pt`、`m0-spike/data/`、`datasets/`、`runs/`、`logs/`、`web/node_modules/`、`web/dist/`、`__pycache__/`、`.pytest_cache/`、`.training.lock`、`quantized_info.csv`（若在根目录则移入 m0-spike/ 后忽略）
4. **CI**（.github/workflows/ci.yml，三 job）：ubuntu-latest；job1 pytest（pip 装 requirements，Linux 版 onnxruntime）；job2 web build（node 20，npmmirror）；job3 ruff check（`pip install ruff && ruff check server/app`，允许初期少量忽略规则但要在配置写明）
5. **本地 git**：`git init` + 首次 `git add -A` 后 `git status --short` 审核（确认无敏感/大文件命中），然后首次 commit（本地分支，**不推远程、不建 remote**）。执行报告列出入库文件总数与未跟踪清单摘要
6. `quantized_info.csv` 归位（若存在）

## 约束

- 不推远程、不改代码逻辑、不删现有文件（移动例外：quantized_info.csv）
- CI 的 pytest 若因训练占用机器而不宜跑，可在本地跳过，标注"CI 配置已验证 YAML 语法（python -c yaml.safe_load）"
- 大文件确认：仓库入库总大小报告（`git ls-files | xargs du` 摘要）

## 验收标准

LICENSE/NOTICE/.gitignore/ci.yml 四文件就位；`git status --short` 干净（无 .env/data/模型/数据）；执行报告含入库文件数、总大小、CI 语法校验结果。
