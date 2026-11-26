# AGENTS.md — LabSentinel 接力开发规范

> 本文件为跨工具/跨模型接力开发的根规则，每次新会话或切换模型时须**全文读取**。

## 1. 仓库与架构基线

- **项目**：LabSentinel（实验室安全视觉监控，Windows 优先）
- **主线仓库**：`D:\User\Desktop\TOOLS\监控识别`（仅保留 `main` 分支，Git 提交者统一为 `windyxyz <siyuan060203@163.com>`）
- **架构路线（ADR-0011 终态）**：
  - **唯一产品运行时**：`engine-rs/`（Rust 单二进制 `labsentinel-server.exe` + DirectML `onnxruntime.dll` + 随包 `yolo11n.onnx` / `yolo11n_ppe.onnx` + `web/` 前端静态资源）
  - **前端**：`web/`（Vue 3 + Vite + Vitest）
  - **开发支具（不进发行包）**：`tools/`（模型训练/预标注，永久保留 Python）、`scripts/`（构建/打包/验收）
  - **已退役/终止**：Python `server/` 已彻底删除；Docker 开发已彻底终止；VM / Flutter / Tauri / 安卓 / CUDA 均不在当前范围

## 2. 接力开发强制协议

### 2.1 会话启动（`relay-start`）
1. 按顺序读取（默认“摘要优先、按需深读”）：
   - `AGENTS.md`（全文）
   - `docs/maintenance/task_registry.md`（当前活跃 `Task-ID` 行）
   - `docs/maintenance/progress.md`（顶部 `Latest Handoff Snapshot`）
   - `docs/maintenance/task_plan.md`（当前 Phase 相关段落）
   - `docs/maintenance/findings.md`（约束、风险与避坑项）
   - `docs/maintenance/修改记录_会话备忘.md`（最近 1~3 条 `Session Record`）
2. 锁定**单一 `Task-ID`**，声明 `Task-Name`。
3. 输出 `Session Context`（Developer/Branch、Task-ID/Task-Name、Active Goal、Current Phase、Next First Command、Open TODO Top3）。
4. 执行启动检查（`Get-Location`、`Get-ChildItem -Force`、`git status --short`、`python -V`），确认后再执行 `Next First Command`。

### 2.2 范围变更（`relay-scope-change`）
需求、约束或优先级变更时，先同步更新 `docs/maintenance/` 里的 `task_plan.md`、`findings.md`、`progress.md` 和 `task_registry.md`，再动代码。

### 2.3 验证红线（`relay-verify`）
- **未执行并记录真实命令与结果前，严禁宣称“已完成/已修复”。**
- 开发批次按改动运行必要的编译检查和定向测试。从 `v0.7.0` 起，每个大版本集中执行全量回归和回归审阅，并覆盖本次大版本累积的小批次。当前 `v0.6.2` 按定向验证收口。
- 大版本 Rust 门禁为 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 和 `cargo test --workspace --all-features`。修改 `Cargo.toml` 时连同 `Cargo.lock` 提交。
- 大版本前端门禁为 `npx vitest run` 与 `npm run build`。接口解包始终校验返回形状（`Array.isArray` 等防御，严禁未校验直接 `.field.length`）。
- 实测结果须回写 `docs/maintenance/progress.md`。

### 2.4 会话收尾（`relay-handoff-stop`）
切换工具/模型或结束会话前，在 `docs/maintenance/` 落盘：
1. 更新 `progress.md` 顶部 `Latest Handoff Snapshot`；
2. 在 `修改记录_会话备忘.md` 追加一条 `Session Record`；
3. 回写 `task_registry.md` 中对应 `Task-ID` 的 `Status`、`Last Update`、`Next First Command`。

## 3. 不可违背的用户决策（摘要）

1. **Rust-first**：后续产品功能与修复一律直接写 Rust + Vue，不再写 Python 产品代码。
2. **GPU 默认 + 跨厂商通用**：推理设备默认 GPU（DirectML / DX12 统一层，适配 N/A/Intel 及核显），失败自动回落 CPU；不做 CUDA 路线。
3. **DirectML 并发红线（事故 A1）**：DirectML `Session::run` 严禁跨线程并发调用（AMD 驱动下会导致 Windows 整机冻结 / Kernel-Power 41）。
4. **产品体验原则**：配置类功能必须在网页内可改并即时生效（不让用户手改配置文件或手动重启）；教程/报错指引需详实可读；浅/深主题都必须使用设计令牌保证高对比度。
5. **发布与安装器原则**：必须产出 `zip` + Inno Setup 7 `setup.exe` 双产物；实验服 PPE 模型随包发布；卸载必须干净（含 `versions/` 与旧目录清理），用户数据默认保留；本地向 GitHub 上传 Release 资产易被代理/S302 劫持，正式发布资产走 CI `release.yml`。

## 4. 本机构建与测试配方（Windows 中文路径必读）

本仓库路径含中文（`D:\User\Desktop\TOOLS\监控识别`），w64devkit 的 `ld.exe` 无法处理非 ASCII 路径下的构建产物：

```powershell
# Rust 构建/测试前必须设置（在 engine-rs/ 目录执行）
$env:PATH = "C:\Users\User\devtools\w64devkit\bin;C:\Users\User\.cargo\bin;C:\Users\User\devtools\node\node-v24.22.2-win-x64;$env:PATH"
$env:CARGO_TARGET_DIR = "C:\Users\User\devtools\ls-target"
$env:RUSTFLAGS = "-L C:/Users/User/.rustup/toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained -L C:/Users/User/devtools/w64devkit/lib -C link-arg=-lucrt"

cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check
cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --all-features -- -D warnings
cargo +stable-x86_64-pc-windows-gnu test --workspace --all-features
```

- **打包（在仓库根目录执行）**：
  - `LS_W64DEVKIT=C:/Users/User/devtools/w64devkit CARGO_TARGET_DIR=C:/Users/User/devtools/ls-target python -X utf8 scripts/build_rust.py --strict --zip`
  - `python -X utf8 scripts/build_setup_exe.py`（Inno Setup 7 路径：`D:\Program Files\Inno Setup 7\ISCC.exe`）
- **前端（在 `web/` 目录执行）**：`npx vitest run`、`npm run build`
- **常见操作坑**：
  - `cargo build` 报 `os error 5` = `labsentinel-server.exe` 进程占用，先 `taskkill /IM labsentinel-server.exe /F`。
  - `cargo test -p ls-server` 不更新 debug 主二进制，端到端调试前须显式 `cargo build -p ls-server`。
  - 跑 debug 二进制时如找不到 ffmpeg，设 `LS_FFMPEG=<repo>/tools/ffmpeg/ffmpeg.exe`。
  - 多行中文源码编辑一律使用文件编辑工具，不用 shell heredoc；发中文 JSON 测试优先用 Python `urllib`，避免 cmd 编码损坏。
