# 任务 54 · 文档口径统一（venv/启动方式/旧命令）

> 背景：项目现在有两条运行路径（项目 `.venv` 源码模式 / `D:\LabSentinel\m0-venv` 旧装兼容 / 便携包 runtime），文档里三种 Python 路径混用；`一键启动.bat` 也已降级为开发模式。新用户按旧文档操作容易踩坑。

## 目标

1. **统一 venv 口径**（全仓 docs/README）：
   - 首选：项目内 `.venv`（deploy.py 创建，`scripts/venv_path.txt` 相对路径）；
   - 兼容：`D:\LabSentinel\m0-venv`（旧装，仅历史/训练环境）；
   - 便携包：`runtime\python.exe`，用户不需要知道 venv。
   - 搜索 `m0-venv`，逐处判断：命令示例改 `.venv` 或标注"旧装兼容"；训练/NPU 相关保留 m0-venv/npu-venv。
2. **启动方式口径**：README/快速开始/部署指南/FAQ 统一为：
   - 普通用户：便携包 `安装.bat`/`启动.bat`；
   - 开发/源码：`一键启动.bat` 或 `scripts/start-all.bat`；
   - 单端口：`npm run build` 后只开 8000。
   把"一键启动"明确标为开发模式，避免普通用户误用。
3. **旧命令核对**：`cd server && m0-venv python -m pytest`、`npm install --registry`、`uvicorn --port 8000` 等逐条对照脚本实际路径；补"如果使用项目 .venv"的等价命令。
4. **链接检查**：新增/修改的相对链接本地可达；不引用不存在的文件。
5. **执行报告**：改动文件、替换数量、核对表、仍保留 m0-venv 的合理场景。

## 约束

- 纯文档；不改代码/脚本/配置；不启停服务；不训练。
- 不删除历史任务报告中的原始命令（历史事实），只改"当前操作指南"类文档（README/快速开始/部署指南/FAQ/scripts README/50-tasks README）。
- 保持中文为主，命令块准确。

## 验收标准

- 当前指南中不再出现"唯一 venv=m0-venv"的误导；
- 三种运行路径叙述一致，命令可复制执行；
- 链接无死链；
- 报告列出替换数量与保留场景。

## 执行报告

**flash 2026-10-29 完成（纯文档，零代码改动）**

### 改动文件与替换统计

| 文件 | 改动 |
|---|---|
| `docs/部署指南.md` | 4 处：§1.3 重写为 `python scripts\deploy.py install` 首选 + `python -m venv .venv` 手动等价（原为在 D:\LabSentinel\m0-venv 建 venv）；§1.5 uvicorn 命令改 `..\.venv`；§1.7 OpenH264 行改"服务端 venv（项目 .venv）"；§1.3 铁律补"训练/量化另用专用环境" |
| `docs/部署FAQ.md` | 1 处：OpenH264 放置路径改为服务端 venv 的 `Lib\site-packages\cv2\`（注明 deploy.py install 自动下载、便携包自带） |
| `android/README.md` | 1 处：服务端运行命令改 `..\.venv`，附部署指南链接 |
| `docs/50-tasks/README.md` | 3 行：Python 环境行改 .venv 首选 + m0-venv 标注"旧装训练专用"；启动服务端/单元测试命令改 `..\.venv`；测试基线"6/6"陈旧数字改"当前基线 91 passed，以最近执行报告为准" |
| `docs/标注与模型迭代手册.md` | 0 替换：开头加"环境口径"说明（m0-venv=训练专用；.venv 可跑抽帧/预标注；便携包用户无需 venv），训练链路命令按规格保留 |

合计替换 9 处 m0-venv 命令口径 + 1 处口径注释 + 1 处过时测试基线修正。

### 启动方式口径核验（三路径叙述一致性）

- 根 README：双路径（便携包推荐/源码）+ `一键启动.bat` 明确标"源码/开发模式"（任务 50 已做，本次核验 ✓）；
- 部署指南 §0 形态表：便携包（推荐入门）/ Windows 原生 / Docker + 单端口模式注记 ✓；
- 部署 FAQ：start-all 端口跳过、venv_path.txt 相对路径说明已有 ✓；
- scripts/README：一键启动定位为"日常入口（体检→自动部署→菜单）" ✓。

### 命令核对表

| 旧命令 | 等价新命令（项目 .venv） | 核对 |
|---|---|---|
| `cd server && D:\LabSentinel\m0-venv\...python.exe -m uvicorn ...` | `cd server && ..\.venv\Scripts\python.exe -m uvicorn app.main:app --port 8000` | .venv 存在且装全依赖（90/91 passed 均用它跑）✓ |
| `python -m venv D:\LabSentinel\m0-venv` + pip | `python scripts\deploy.py install`（自动 venv+依赖+组件+venv_path.txt） | deploy.py install 流程实测过（新机冷部署）✓ |
| `m0-venv python -m pytest` | **维持 m0-venv**：pytest 是训练环境工具，`.venv` 未装（实测 `No module named pytest`）；如需在 .venv 跑测试先 `pip install pytest` | 本次实测 91 passed（m0-venv）✓ |

### 仍保留 m0-venv 的合理场景（未改）

1. 训练/导出/评估链路：`tools/README.md`（工具链总表）、`docs/标注与模型迭代手册.md`（训练命令）、
   `docs/部署FAQ.md` 训练命令段——ultralytics/torch 只装在 m0-venv（Py3.14），属训练专用环境；
2. NPU 量化：`npu-venv`（Py3.12，amd-quark），本就独立；
3. 历史报告（不改动）：50-tasks/ 各任务执行报告、40-roadmap、51-loadtest、53-v2-demo、
   55-handoff、56-flywheel、62-复审、64-usb、发布核对报告——命令是当时事实记录；
4. 环境事实类：numpy 钉死、onnxruntime 互斥恢复流程等铁律段落（描述的就是 m0-venv/npu-venv 本身）。

### 链接检查

新增/改动链接 9 处全可达：部署指南→tools/README（相对 `../tools/README.md`）、
android/README→docs/部署指南（`../docs/部署指南.md`）、标注手册既有链接等，逐一 `ls` 核实存在 ✓。
