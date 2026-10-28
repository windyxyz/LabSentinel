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

（待填：改动文件 / m0-venv 替换统计 / 命令核对 / 链接检查 / 保留场景）
