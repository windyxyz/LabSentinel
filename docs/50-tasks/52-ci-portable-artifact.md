# 任务 52 · CI 便携包构建流水线（Windows artifact + sha256）

> 背景：便携包现在是手工 `python scripts/build_portable.py --zip`。要像"官方安装包"那样稳定分发，需要在 CI 里自动构建、冒烟、产出校验和与 artifact。仓库当前是本地 git（无远端），本任务先把 workflow 写好并做本地静态验证。

## 目标

1. 新增 `.github/workflows/portable.yml`：
   - 触发：`workflow_dispatch` + `push tags: v*`（不在每次 push 构建，节省分钟）。
   - `runs-on: windows-latest`；步骤：
     1) `actions/checkout@v4`
     2) `actions/setup-python@v5`（3.12）+ 缓存 pip
     3) `actions/setup-node@v4`（20）+ 缓存 npm
     4) `pip install -r server/requirements.txt`（清华源）
     5) `npm ci --registry=...` + `npm run build`
     6) `python scripts/build_portable.py --zip`
     7) 计算 `Get-FileHash` SHA256 写 `dist/SHA256SUMS.txt`
     8) `actions/upload-artifact@v4` 上传 zip + SHA256SUMS
   - `timeout-minutes: 30`；失败时上传 `dist/portable` 日志（可选）。
2. **本地静态验证**：
   - 用 Python `yaml.safe_load` 或 `pip install pyyaml` 解析 workflow，确认语法；
   - 逐条核对命令与本机脚本/要求一致（python 3.12、node 20、build_portable --zip、license 文件存在）；
   - 不实际运行 GitHub Actions（无远端/无法触发），报告写明"静态验证 + 本地等价命令已跑过"。
3. **文档**：`docs/70-release-plan.md` 或 `docs/59-portable-install.md` 增加"CI 产物/校验和验证方法"一小节
   （下载 zip → `Get-FileHash` 对比 SHA256SUMS）。
4. **执行报告**：YAML 解析结果、命令核对表、未实测项（无远端）、后续启用 GitHub 时需要做的设置（如 Actions 权限/发布 token）。

## 约束

- 只新增 workflow 与文档；不改业务代码；不引入 secrets；不在本地跑完整 CI。
- 构建步骤必须与本地 `build_portable.py` 一致，不另写一套打包逻辑。
- 不把 `dist/` 提交进仓库。

## 验收标准

- workflow YAML 可解析；触发/权限/超时/artifact 路径正确；
- 命令核对表全部对应现有脚本；
- 文档含校验和使用方法；
- 报告如实说明"未在真实 GitHub runner 跑过"。

## 执行报告

（待填：workflow 路径 / YAML 解析 / 命令核对 / 文档 / 未实测项）
