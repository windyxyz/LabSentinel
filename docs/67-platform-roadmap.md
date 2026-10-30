# 67 · 平台路线图（Windows 优先）

> 决策（2026-10-31，用户）：**先把 Windows 版本开发完并发布**；Linux、Android、iOS/鸿蒙
> 放到后续阶段。WSL Fedora **只作为开发/调试工具**，不是当前交付目标。
> **Docker 暂不考虑**：不派发 Docker/Compose 验证/打包任务，文档不承诺容器部署。

## 阶段顺序

| 阶段 | 目标 | 进入条件 | 状态 |
|---|---|---|---|
| **P0 Windows 功能完成** | 补齐分组/日期规则/发现/守护/PWA/模板/更新检查；修完 105–110；重建便携包 | 当前 | 进行中 |
| **P1 Windows 发布候选** | 真实 RTSP 24h、模型止损线、干净 VM 安装/卸载/自启、Release 资产 | P0 全绿 + 用户摄像头/数据 | 阻塞 |
| **P2 Linux 部署（可选）** | CI Linux job、systemd/发行版文档；**不含 Docker** | Windows 发布候选通过；用户确认有 Linux 部署需求 | 未开始 |
| **P3 安卓 APK** | Flutter 构建/签名/WS 通知；真机联调 | 用户提供测试手机/开发者账号 | 未开始 |
| **P4 iOS/鸿蒙/桌面壳** | 视需求评估 | 明确需求 | 未开始 |

## Linux/WSL 的定位

- 当前仅用于：在 WSL Fedora（`FedoraLinux-44`，Python 3.14.3）跑纯 Python 单测/脚本兼容性调试、
  验证跨平台路径/依赖问题；**不产出 Linux 发行包**。
- **Docker/Compose 暂不考虑**：不派发相关任务；仓库里的 `Dockerfile`/`docker-compose.yml`
  保留作历史/参考，但**不实测、不宣传、不支持**，README/部署指南不把容器作为可用路径。
- 后续 P2 的启动条件：Windows 版发布候选通过 + 用户明确 Linux 部署需求（届时再定 systemd/发行版）。

## 对现有文档的影响

- `README` 的部署形态只承诺 Windows 便携包/源码；Docker 一节标注“实验性，未实测”。
- `docs/60-milestones.md` 的 M4 多端顺序按本文件调整：Windows → Linux → Android → iOS/鸿蒙。
- 任务 119/120（WSL 回归/Docker 验证）**本阶段不派发**；需要时再开。

## 平台无关的代码约束（现在就要遵守）

- 路径用 `pathlib`，不硬编码 `C:\`；文件编码显式 UTF-8；
- Windows 专属（C# 启动器、注册表、DPAPI、任务计划）放在 `scripts/launcher_exe`/`install_portable`，
  不污染 `server/app` 业务层；
- 服务端不得 import Windows-only 模块（ctypes.windll 等）于模块顶层；必须 try/except 或惰性调用；
- SQLite 路径/URI 用统一 helper（任务 76），避免 Linux 下空格/中文差异；
- 新增依赖写清平台 marker（参考 requirements 的 onnxruntime/directml 写法）。
