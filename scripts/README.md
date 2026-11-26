# scripts/ · 构建 / 验收 / 运维脚本

> RA4.2（ADR-0011）：Python 运行时（原 `server/`）与 Python 便携打包链已退役。
> 本目录的 Python 脚本只服务**构建、验收与开发**，不是产品运行时依赖。
> 最终用户只需要发布包里的 `LabSentinel.bat`。

## 发布 / 构建

| 脚本 | 用途 |
|---|---|
| `prepare_rust_assets.py` | 准备发行资产：模型（基础+PPE+类别名）、ffmpeg/ffprobe、onnxruntime DLL（哈希锁） |
| `build_rust.py` | 构建 Rust 单二进制发行包；`--strict` 资产齐全才出包，`--zip` 打 zip |
| `sign_windows_release.ps1` | 签名并验证 Windows 主程序，组装 ZIP 和安装器，验证安装器签名后生成 SHA-256 清单 |
| `smoke_rust.py` | 发行包隔离冒烟（解包 → 启动 → 就绪 → API/前端 → 停止） |
| `fetch_assets.py` | 按 `assets.lock.json` 下载并 SHA256 校验外部资产 |
| `bump_version.py` | 版本号单源同步（`VERSION` → `web/package.json`） |
| `rust_route_coverage.py` | 核对运行程序与 OpenAPI 的路由登记（`--exe`/`--strict`） |
| `make_update_manifest.py` | 生成应用内更新检查 manifest（可 minisign 签名） |
| `清理重复验收缓存.ps1` | 按 SHA-256 查找历史验收归档中的重复二进制、权重和测试视频副本 |

在 GitHub Actions 手动运行 `release` 可以在干净的 Windows 运行器验证个人安装、原生服务和卸载。产物位于这次运行的 `labsentinel-windows-acceptance`，供验收使用。

在项目根目录运行 `powershell.exe -NoProfile -ExecutionPolicy RemoteSigned -File ".\scripts\清理重复验收缓存.ps1"` 可预览。加 `-List` 显示删除和保留的路径。确认后加 `-Apply` 执行。脚本仅处理 `Resources/验收归档/` 的重复产物，预览显示原始文件体积。

发布标签走 `release-signing` 环境。将该环境限制为经过审阅的发布标签，并配置审批。专用 Windows x64 签名运行器使用 `labsentinel-signing` 标签，需要 PowerShell 7、Python、Inno Setup 7 和 Windows SDK 的 SignTool。签名账户需能访问当前用户证书库中的代码签名证书及其私钥。在 `release-signing` 的 Variables 中将证书指纹设为 `LS_SIGNING_THUMBPRINT`。校验双产物及签名后才会创建 Release 草稿。

## 验收 / 测试

| 脚本 | 用途 |
|---|---|
| `acceptance_rust_only.py` | 零 Python 部署端到端验收（39 步：相机/区域/规则/模板/日历/folder worker/统计/导出/配对/插件/模型/VLM 降级等） |
| `acceptance_vlm.py` | 外部联网模型（VLM）通路验收：mock 端点 + 真实素材，openai_compat 帧协议/回填/事件全链 |
| `mock_vlm_server.py` | 上述验收的 OpenAI 兼容 mock 模型端（确定性启发式判定） |

## 运维 / 开发

| 脚本 | 用途 |
|---|---|
| `start-all.bat` / `stop-all.bat` | 源码开发三组件启停（go2rtc 1984 / Rust API 8000 / Vite 5173） |
| `dispatch_tick.py` | 主线/子代理看板派发 tick（内部协作用，不随产品分发） |

## 说明

- 退役清单（保留在 git 历史中）：`build_portable.py`、`install_portable.py`、`installer.py`、
  `portable_launcher.py`、`smoke_portable.py`、`deploy.py`、`doctor.py`、`health_check.py`、
  `backup_data.py`、`seed_demo.py`、`export_openapi.py`、`release_manifest.py`、
  `release_check.py`、`rc_regression.py`、`probe_runtime.py`、`secrets_rekey.py`、
  `verify_sidecar_evidence.py`、`opentip_hashes.py`、`make_sbom.py`、`soak_mixed_gray.py`、
  `build_launcher_exe.py` 及 `server/` Python 服务端。
- 备份/恢复/诊断/模型下载等人机功能现在都由 Rust 控制面提供（网页「设置」「模型管理」）。
- `.bat` 按 **GBK/ANSI** 保存（cmd OEM 代码页）；Python 脚本 UTF-8。
