# 59 · 便携版安装 / 升级 / 卸载指南（任务 46）

> 适用：`dist/LabSentinel-portable-*.zip` 解压后的目录（含 `runtime\` 嵌入式 Python，无需安装 Python/Node）。
> 全程**不需要管理员权限**：只写用户目录与 HKCU 注册表。

## 安装

1. 解压便携包到任意目录（下称"包目录"）。
2. 双击包目录里的 **`安装.bat`**（内容为纯 ASCII；`runtime\python.exe` 运行安装器）。
3. 完成后：
   - 程序复制到 `%LOCALAPPDATA%\Programs\LabSentinel\`
   - 桌面 + 开始菜单生成 `LabSentinel.lnk`（优先指向 **`LabSentinel.exe`**——无控制台的
     应用程序入口；包内无 exe 时回退 `runtime\pythonw.exe` 启动器，同样无黑框）
   - 开始菜单另有 `卸载.lnk`
4. 双击桌面快捷方式，或包目录里的 **`LabSentinel.exe`**（或 `启动.bat`）→ 等约 10 秒 →
   浏览器自动打开控制面板。

> `LabSentinel.exe` 是 C# 无控制台启动器（.NET Framework 4.x，Win10/11 自带，无需安装），
> 只负责拉起 Python 启动器；单实例/端口/健康检查/开浏览器逻辑全部在 Python 启动器里，
> 两种入口行为完全一致。`LabSentinel.exe --stop` 亦可停止服务。

### 开机自启（可选）

```bat
runtime\python.exe scripts\install_portable.py install --source . --autostart
```
写入 `HKCU\...\Run\LabSentinel`；重装不带 `--autostart` 即关闭。
> 注意：部分安全软件/启动项管理工具会锁定 Run 键（写入报"拒绝访问"）——安装器会如实提示，
> 此时改用"shell:startup 放快捷方式"的方式自启。

### 静默安装 / 自定义目录 / 批量部署（任务71）

| 场景 | 命令（在包目录内执行） |
|---|---|
| 静默安装（无交互，结果仍打印） | `runtime\python.exe scripts\install_portable.py install --source . --silent` |
| 安装到自定义目录 | `... install --source . --install-dir "D:\Apps\LabSentinel"` |
| 不生成快捷方式 | 加 `--no-shortcuts` |
| 明确不开自启 | 加 `--no-autostart`（与 `--autostart` 互斥） |
| 批量部署预检 | 加 `--dry-run`（只打印动作，不写文件/注册表） |
| 卸载指定目录 | `... uninstall --install-dir "D:\Apps\LabSentinel"`（`--purge-data` 连数据一起删） |

安全边界：拒绝安装到 Windows 目录/盘符根/用户主目录/LOCALAPPDATA 根；覆盖非 LabSentinel 的
非空目录需 `--force`。数据目录始终在 `%LOCALAPPDATA%\LabSentinel`（安装到任何位置都不变）。

## 数据位置（升级/卸载都不丢）

| 内容 | 路径 |
|---|---|
| 数据库/截图/视频/标注池 | `%LOCALAPPDATA%\LabSentinel\data\` |
| 运行日志 + 启动锁 | `%LOCALAPPDATA%\LabSentinel\logs\` |
| 端口等用户配置 | `%LOCALAPPDATA%\LabSentinel\config.env` |

> **磁盘水位（任务68）**：仪表盘「磁盘」卡实时显示剩余空间；剩余低于 `config.env` 可配的
`LS_DISK_MIN_FREE_GB`（默认 5GB）时变红并提示将触发紧急清理（从最旧事件删起）。
`GET /api/maintenance/preview` 可只读预演清理将删除的内容。

> **WAL 说明（任务64）**：`data\` 下会有 `labsentinel.db-wal`/`-shm` 两个伴随文件——**属正常，
> 不要手删**；正常停止服务会自动合并。备份/迁移务必用下面的 backup 命令（SQLite backup API），
> **只拷 `.db` 文件会丢 WAL 中未合并的数据**。

日志明细（任务 61）：`launcher-<日期>.log` 为启动器日志；`server.log` 为服务端日志
（5MB×4 份轮转，业务模块错误也落这里；uvicorn 的 access 日志仍只走控制台）。
报障时可一键生成脱敏诊断包：

```bash
runtime\python.exe scripts\collect_diagnostics.py --out diag.zip --data-dir "%LOCALAPPDATA%\LabSentinel\data"
```

数据备份/迁移（任务 62，**运行中可直接备份**，SQLite 一致快照）：

```bash
runtime\python.exe scripts\backup_data.py backup --data-dir "%LOCALAPPDATA%\LabSentinel\data" --out backup.zip
runtime\python.exe scripts\backup_data.py verify --zip backup.zip
runtime\python.exe scripts\backup_data.py restore --zip backup.zip --data-dir <目标data目录> --force
```

## 首次运行自动播种（演示数据）

首次启动若库为空，启动器自动播种 **4 路演示摄像头**（包内 `media\` 两段演示视频循环播放）
+ 每路安全区与 enter/enter_no_ppe 规则——打开仪表盘即可看到"开箱即用"的效果。
- 只在**空库**时播种；你添加过摄像头/有事件后，重装/升级都不会动你的数据。
- 想清空回演示状态：停止服务（`停止.bat`）→ 删除 `%LOCALAPPDATA%\LabSentinel\data\labsentinel.db`
  → 再次启动即可重新播种。
- 包内缺 `media\` 时跳过播种（只警告，不阻塞启动）。

## 端口

默认 8000；被占用时启动器自动从 8100–8199 选一个可用端口并在 `config.env` 写回，
浏览器打开的是**实际端口**。手工指定：编辑 `config.env` 的 `port=`。

## 升级

新版本便携包解压后，直接再跑一次新包的 `安装.bat`——覆盖程序文件，
`%LOCALAPPDATA%\LabSentinel`（数据/配置/日志）不动。

> **数据库自动迁移（任务69）**：升级后首次启动会自动把数据库迁移到新版结构（只加列/表，不动数据）。
> 建议升级前先备份（任务 62 的 backup 命令，`runtime\python.exe scripts\backup_data.py ...`）。
> 若启动报「数据库迁移 vN 失败」，保留该提示截图并反馈（先备份再操作）。

## 卸载

- 开始菜单 `卸载.lnk` 或安装目录 `卸载.bat`
- 流程：停止服务 → 删快捷方式与自启键 → **询问是否删除数据（默认保留）** → 删除安装目录
- 数据保留时会打印路径；确认不要可手动删，或重跑 `卸载.bat` 选 Y（等价 `--purge-data`）

## 卸载细节（技术说明）

- 安装目录自删采用"哨兵守卫延迟 rd"：卸载进程运行在 `runtime\python.exe` 上，自身目录无法
  即时删除；进程内先删 `app/models/tools/...`（**绝不碰 runtime**——卸载器解释器的惰性 stdlib
  从 `runtime\python312.zip` 加载，删了会当场崩，实测 04:33），再对带 `runtime\PENDING_DELETE`
  哨兵的目录发起后台 rd 重试链（约 3 分钟视距）。重装的新目录没有哨兵，绝不会被晚到的 rd 误删。

## 发布产物清单 / 校验和验证（任务 52/58）

正式发布时由 GitHub Actions（`.github/workflows/portable.yml`，触发：推 `v*` 标签或手动
workflow_dispatch）在 `windows-latest` 上自动执行 `python scripts/build_portable.py --zip`，
artifact 名 `LabSentinel-portable`，内含便携包 zip 与 `SHA256SUMS.txt`（保留 14 天）。

**本地构建的核对**（任务 58，`scripts/release_manifest.py`，纯标准库）：

```bash
python scripts/release_manifest.py          # 对 dist/ 最新 zip 生成 SHA256SUMS.txt + RELEASE-MANIFEST.json
python scripts/release_manifest.py --verify dist   # 重算 sha256/大小对比 manifest，一致打印 OK
```

- 必需条目缺失 → 退出码 2 并列出 `missing_required`；可选条目（`LabSentinel.exe`/`卸载.bat` 等）缺失只警告；
- 下载方校验（PowerShell）：`Get-FileHash .\LabSentinel-portable-<版本>.zip -Algorithm SHA256`
  与 `SHA256SUMS.txt` 对比，或直接用发布页附带的 `RELEASE-MANIFEST.json` 核对
  version/size/sha256/条目数。

**一键冒烟**（任务 59，发布前固定关卡；完全隔离，不碰真实数据）：

```bash
python scripts/smoke_portable.py --zip dist/LabSentinel-portable-<版本>.zip
# 输出 8 项 PASS/FAIL（隔离环境/启动/就绪/前端/播种/停止/隔离断言/清理），全绿 exit 0
```

沙箱数据在临时目录（LOCALAPPDATA 重定向），对 `--zip` 不存在的文件、包内无 `LabSentinel.exe`
（自动回退 `启动.bat`）等反例有明确处理；失败保留现场并打印路径。

> 注意：模型文件（`m0-spike/*.onnx`）与部分 tools 二进制不随仓库分发，CI checkout 后
> `build_portable.py` 的资产断言会失败。真实启用流水线前需先解决模型进 CI 的通道
> （Git LFS / artifact 传递 / 发布时临时提交），详见任务 52 执行报告。

## 启动失败先自检（任务70）

双击没反应/服务起不来时，先跑只读自检（不启动服务、不加载模型）：

```bat
runtime\python.exe scripts\doctor.py
```

10 项检查逐条给出 PASS/WARN/FAIL 与修复建议（运行时/程序文件/模型/媒体工具/配置/数据库完整性/
端口/磁盘/写权限）；支持 `--json`（支持脚本用）与 `--port`。FAIL 时按建议处理后再启动。

## FAQ

- **双击 启动.bat 闪退**：看 `%LOCALAPPDATA%\LabSentinel\logs\launcher-*.log`（pythonw 的异常
  也会落在这里——启动器顶层有异常落盘）。
- **杀软弹窗/误报**：嵌入式 Python + onnxruntime 直调 DLL 属常见误报特征，加入白名单即可；
  本包无数字签名（开源项目签名的成本说明见 README）。
- **无网络**：安装/运行均不需要网络（依赖已预装）；仅告警推送需要出网。
- **端口 8000 被占**：自动换 8100–8199，`config.env` 可查实际端口。
- **快捷方式双击没反应**：确认 `runtime\pythonw.exe` 存在；查看 launcher 日志。
