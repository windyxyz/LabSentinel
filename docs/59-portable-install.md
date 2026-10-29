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

## 数据位置（升级/卸载都不丢）

| 内容 | 路径 |
|---|---|
| 数据库/截图/视频/标注池 | `%LOCALAPPDATA%\LabSentinel\data\` |
| 运行日志 + 启动锁 | `%LOCALAPPDATA%\LabSentinel\logs\` |
| 端口等用户配置 | `%LOCALAPPDATA%\LabSentinel\config.env` |

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

## 卸载

- 开始菜单 `卸载.lnk` 或安装目录 `卸载.bat`
- 流程：停止服务 → 删快捷方式与自启键 → **询问是否删除数据（默认保留）** → 删除安装目录
- 数据保留时会打印路径；确认不要可手动删，或重跑 `卸载.bat` 选 Y（等价 `--purge-data`）

## 卸载细节（技术说明）

- 安装目录自删采用"哨兵守卫延迟 rd"：卸载进程运行在 `runtime\python.exe` 上，自身目录无法
  即时删除；进程内先删 `app/models/tools/...`（**绝不碰 runtime**——卸载器解释器的惰性 stdlib
  从 `runtime\python312.zip` 加载，删了会当场崩，实测 04:33），再对带 `runtime\PENDING_DELETE`
  哨兵的目录发起后台 rd 重试链（约 3 分钟视距）。重装的新目录没有哨兵，绝不会被晚到的 rd 误删。

## CI 产物 / 校验和验证（任务 52）

正式发布时由 GitHub Actions（`.github/workflows/portable.yml`，触发：推 `v*` 标签或手动
workflow_dispatch）在 `windows-latest` 上自动执行 `python scripts/build_portable.py --zip`，
artifact 名 `LabSentinel-portable`，内含便携包 zip 与 `SHA256SUMS.txt`（保留 14 天）。

下载后校验完整性（PowerShell）：

```powershell
Get-FileHash .\LabSentinel-portable-<版本>.zip -Algorithm SHA256
# 输出的 Hash 与 SHA256SUMS.txt 中该文件名对应的哈希一致即为完整包
```

> 注意：模型文件（`m0-spike/*.onnx`）与部分 tools 二进制不随仓库分发，CI checkout 后
> `build_portable.py` 的资产断言会失败。真实启用流水线前需先解决模型进 CI 的通道
> （Git LFS / artifact 传递 / 发布时临时提交），详见任务 52 执行报告。

## FAQ

- **双击 启动.bat 闪退**：看 `%LOCALAPPDATA%\LabSentinel\logs\launcher-*.log`（pythonw 的异常
  也会落在这里——启动器顶层有异常落盘）。
- **杀软弹窗/误报**：嵌入式 Python + onnxruntime 直调 DLL 属常见误报特征，加入白名单即可；
  本包无数字签名（开源项目签名的成本说明见 README）。
- **无网络**：安装/运行均不需要网络（依赖已预装）；仅告警推送需要出网。
- **端口 8000 被占**：自动换 8100–8199，`config.env` 可查实际端口。
- **快捷方式双击没反应**：确认 `runtime\pythonw.exe` 存在；查看 launcher 日志。
