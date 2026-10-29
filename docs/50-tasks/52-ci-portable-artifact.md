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

**flash 2026-10-29 完成（静态验证通过；未在真实 GitHub runner 运行）**

### 交付物

- `.github/workflows/portable.yml`：name `portable`，单 job `build-portable`，9 步：
  1. `actions/checkout@v4`
  2. `actions/setup-python@v5`（3.12，`cache: pip`）
  3. `actions/setup-node@v4`（20，`cache: npm`）
  4. `pip install -r server/requirements.txt -i 清华源`
  5. `web/` 内 `npm ci --registry=npmmirror` + `npm run build`
  6. `python scripts/build_portable.py --zip`（与本地构建完全同一命令，无第二套打包逻辑）
  7. `Generate SHA256SUMS`（pwsh：取最新 `dist/LabSentinel-portable-*.zip` →
     `Get-FileHash` 小写哈希 → `dist/SHA256SUMS.txt` ASCII 编码，格式 `hash  文件名`）
  8. `actions/upload-artifact@v4`：`LabSentinel-portable`，zip+SUMS，
     `if-no-files-found: error`，`retention-days: 14`
  9. `if: failure()` 上传 `dist/portable/**/*` 构建日志（`if-no-files-found: ignore`）
- 顶层 `permissions: contents: read`（最小权限，无 secrets）；`timeout-minutes: 30` 在 job 层
  （曾误放在 steps 层导致解析为未知键，已修正）。
- 触发：`workflow_dispatch` + `push tags: ["v*"]`——不在每次 push 构建，节省 Actions 分钟数。

### 静态验证（D:\tmp\check_ci.py，venv python + pyyaml，全绿）

- YAML `safe_load` 解析成功，job/steps 结构完整；
- 命令核对 5 项 ✓：requirements 路径、`npm ci`、`npm run build`、`build_portable.py --zip`、
  `Get-FileHash`；
- `uses:` 字段核对 4 个 action 版本 ✓（checkout@v4 / setup-python@v5 / setup-node@v4 /
  upload-artifact@v4）——**第一版核对脚本漏扫 uses 字段导致 upload-artifact 断言失败，
  属核对脚本问题非 workflow 问题**，已补 uses 扫描 + with 参数断言
  （python/node 版本、artifact 严格模式、保留期）+ 失败日志步骤检查；
- 触发器正则、依赖文件存在（build_portable.py / requirements.txt / web/package.json /
  LICENSE / NOTICE）✓。

### 未实测项（如实说明）

1. **未在真实 GitHub runner 运行过**：仓库当前是本地 git 无远端，无法触发 workflow。
   本地等价命令（pip install → npm build → build_portable --zip → Get-FileHash）均在本机
   实际跑通过（任务 46/51 便携包重建即同一链路），但 CI 环境差异（runner 版本、缓存行为、
   shell 差异）未验证。
2. **启用前提（关键阻塞）**：模型文件 `m0-spike/*.onnx` 与 tools 下的 ffmpeg/ffprobe/
   OpenH264 等二进制**不随仓库分发**（体积/许可原因）。CI checkout 后 `build_portable.py`
   的资产断言会失败，流水线第一步构建便携包就挂。真实启用需三选一：
   - 模型入 Git LFS（需要仓库启用 LFS，Actions checkout 默认不拉 LFS，需
     `lfs: true` 参数）；
   - 模型走单独 artifact/Release 下载步骤注入；
   - 发布时临时提交模型（不推荐，历史膨胀）。

### 后续启用 GitHub 时需要做的设置

1. 推送仓库到 GitHub（含 `.github/workflows/portable.yml` 与本 workflow 引用的全部脚本）；
2. 解决上述模型进 CI 通道（推荐 LFS + checkout `with: lfs: true`）；
3. Actions 默认权限已够（workflow 只声明 `contents: read`）；如要把 artifact 转正式
   Release，需追加 `actions/upload-release-asset` 或用 `gh release create`（GITHUB_TOKEN
   写 contents 权限需另行声明）；
4. 私有仓库注意 Actions 分钟数配额（windows runner 计费系数 2x，30 分钟超时上限即
   最多消耗 60 计费分钟/次）；
5. exe 代码签名（任务 55 产物）未接入——无签名 zip 在部分浏览器会告警，记录为发布项。

### 文档

- `docs/59-portable-install.md` 新增"CI 产物 / 校验和验证"小节：下载 → `Get-FileHash`
  对比 SHA256SUMS 的操作方法 + 模型不入库的启用前提提示。
