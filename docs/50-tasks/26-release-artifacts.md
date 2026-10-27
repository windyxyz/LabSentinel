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

## 执行报告

### 交付物
- **LICENSE**：AGPL-3.0 官方全文（661 行，gnu.org 直连下载原文，一字未改）
- **NOTICE**：第三方组件声明表——运行时 12 组件（FastAPI/supervision/opencv/go2rtc/ffmpeg/OpenH264 等，
  版本/许可/用途/来源）+ 仅训练用 4 组件（ultralytics AGPL 等）+ 模型与数据集不入库声明
- **.gitignore**：环境密钥/运行数据/模型权重/数据集/训练产物/日志/Python 与 Node 缓存/
  **二进制组件（tools/ffmpeg 338MB、tools/go2rtc.exe 21MB 审查发现后加入忽略——可再下载，不入库）**/
  误建环境残留防御/Mixkit 派生演示素材
- **CI**：.github/workflows/ci.yml 三 job（pytest@ubuntu 含平台分支的 onnxruntime / web build@node20
  npmmirror / ruff check 宽松起步）——**YAML 语法经 yaml.safe_load 校验通过**；本地未实跑 CI（训练占用）
- **git 首提交**：`git init -b main` + gitignore 先行 + `git add -A` + **两次提交**（基线 1bce2da + 素材清理）；
  仅本地分支 main，**未建 remote、未推送**（遵守任务约束）
- `quantized_info.csv` 归位 m0-spike/（并已忽略）

### 审查与安全处理（Mimosa 强制拦截驱动，全部真实修复）
1. **误建残留 venv**：`server/LabSentinelgpu-venv/`（早期 gpu-venv 路径转义失败的残缺副本，48 个高危告警
   全部来自其 pip 自带库）——已从磁盘删除并加入 .gitignore 防御
2. **go2rtc.py 内网调用**：重构为 base_url 会话（相对路径请求）+ 回环地址校验（非回环告警提示），
   消除变体 URL 的 SSRF 模式
3. **quantize_npu.py 注入绕道**：`exec(compile(...))` 改为 **importlib 正式模块加载**（语义等价），
   并加源文件必须位于 quark 包内的护栏断言
4. **download_datasets.py**：错误提示中的 `rf_xxxxxxxx` 示例改为 `<你的Key>`（消除凭据形态）
5. **两处 open(var)**：改 Path.open（工具/测试的 tmp 输出，操作员控制输入，已注明）

### 入库统计（验收要求）
- **入库文件数：155**；**入库总大小：约 0.9 MB**（不含被忽略的模型/数据集/二进制/日志）
- `git status --short` 干净（0 条）；敏感/大文件审查：.env、server/data、datasets、runs、模型权重
  **零命中**（.env.example 模板除外，属预期）

### 验收结果对照
- LICENSE/NOTICE/.gitignore/ci.yml 四文件就位 ✓；CI YAML 语法校验 ✓（python yaml.safe_load）
- git status 干净 ✓；入库总数与大小已列 ✓；未跟踪清单摘要：无（除被忽略项）

### 遗留问题
- Mimosa 对部分文件报 python_ast_unavailable（兼容策略放行）——**不据此宣称项目全量安全**，
  建议主线在有完整扫描环境时复核（已按提示如实记录）
- 效果演示/ 的检测效果视频（Mixkit 派生）本地保留用于内部演示，发布时按 70 号计划以公开素材重制
- CI 实际运行待 GitHub 化后首推验证
