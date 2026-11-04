# scripts/ · 当前有效的构建/安装/运维脚本

> 2026-11 清理后只保留**当前入口**。历史任务文档仍在 `docs/50-tasks/`，仅供追溯，不再作为操作入口。

## 用户/发布入口

| 入口 | 用途 |
|---|---|
| `../安装.bat`（便携包内） | 调 `installer.py install --ui`：首启三选、目录选择、自启、卸载项 |
| `../启动.bat` / `LabSentinel.exe`（便携包内） | 便携启动（内部调 `portable_launcher.py`） |
| `../一键启动.bat`（源码根） | 开发模式：`deploy.py check/install` → 启动 → 开面板 |
| `installer.py` | 安装器交互 UI（安装/卸载/三选） |
| `install_portable.py` | 安装/卸载引擎（do_install/do_uninstall，供 installer 复用） |
| `portable_launcher.py` | 便携启动器：单实例、端口、健康等待、开浏览器、`--stop` |
| `portable_start.bat` / `portable_stop.bat` | 安装位置/便携位置智能定位启停 |

## 构建/发布

| 脚本 | 用途 |
|---|---|
| `build_portable.py` | 构建便携运行时并 `--zip` 打包 |
| `build_launcher_exe.py` | 用系统 csc 编译 `LabSentinel.exe` |
| `release_manifest.py` | 对 zip 生成 `SHA256SUMS.txt` + `RELEASE-MANIFEST.json`，`--verify` |
| `smoke_portable.py` | 便携包隔离冒烟（解压→启动→健康/播种/前端→停止→清理） |
| `doctor.py` | 只读自检（模型/Web/DB/端口/磁盘/配置） |
| `release_check.py` | 发布终检（链接/密钥/TODO/必需文件/manifest） |
| `rc_regression.py` | RC 快速回归编排（pytest/build/release_check/doctor） |
| `export_openapi.py` | 导出/校验 `docs/openapi.json` 快照 |

## 数据/运维

| 脚本 | 用途 |
|---|---|
| `backup_data.py` | SQLite 在线备份/校验/恢复 |
| `collect_diagnostics.py` | 收集脱敏诊断 zip |
| `seed_demo.py` | 首次启动播种演示摄像头（安装/便携共用） |
| `deploy.py` | 源码模式环境体检/安装/重部署/卸载 |
| `start-all.bat` / `stop-all.bat` | 源码开发三组件启停（go2rtc 1984 / API 8000 / Vite 5173） |
| `dispatch_tick.py` | 主线/子代理看板派发 tick（内部协作用，不随产品分发） |

## 说明

- 已删除：`make_demo_zip.py`（旧 demo 分发包，已被 `build_portable.py` 取代且引用失效）、
  `health-check.bat`（被 `doctor.py`/浏览器健康检查取代）。
- 所有 `.bat` 仍按 **GBK/ANSI** 保存（cmd OEM 代码页）；Python 脚本 UTF-8。
- 发布物只由 `build_portable.py` 生成，不要再手工拼 zip。
