# 任务 75 · 便携包发布回归修复（P0，主线验收发现）

> 背景：57–74 夜间批次主体验收通过（pytest 165），但主线复核发现 4 个便携包/发布链真实缺口。
> 本任务只修这些，不重做已完成功能；修完必须**重建 zip 并重跑发布链**。

## 必须修复（按优先级）

### 75.1 USB 枚举在便携包布局下找不到 ffmpeg

- 现状：`server/app/usb_devices.py` 的 `FFMPEG = Path(__file__).resolve().parents[2] / "tools" / "ffmpeg" / "ffmpeg.exe"`。
  源码布局成立；便携包为 `app/server/app/usb_devices.py`，该式指向 `app/tools/ffmpeg/ffmpeg.exe`，
  而包内实际是**扁平** `tools/ffmpeg.exe`（`build_portable.py` 复制到 `DIST/tools/ffmpeg.exe`）。
  → 便携版 `/api/cameras/usb-devices` 会返回“未找到 ffmpeg”。
- 要求：
  - 新增 `resolve_ffmpeg(root: Path) -> Path`（纯函数，便于测试），候选顺序：
    环境变量 `LS_FFMPEG`（存在才用）→ `root/tools/ffmpeg/ffmpeg.exe` →
    `root/tools/ffmpeg.exe` → （如有）`root/ffmpeg.exe`；返回第一个存在者，都不存在返回第一个候选用于报错。
  - 模块按“源码根/便携包根”正确推导 root：从 `__file__` 向上找到同时含 `server`/`app` 或 `runtime` 的目录；
    不要写死 parents 下标。保留模块级 `FFMPEG` 变量（测试会 monkeypatch），但 `list_devices()` 每次用 resolver
    或 `LS_FFMPEG` 重新解析，避免缓存错误路径。
  - `error` 文案同时列出尝试过的路径（最多 3 条）。
  - 测试：源码布局、便携布局（`tmp/runtime/python.exe` + `tmp/tools/ffmpeg.exe`）、`LS_FFMPEG` 覆盖、
    全缺失时 error 含路径；更新 `test_usb_devices.py` 的 monkeypatch 方式。
  - **105 后复核实测补充（必须修）**：`detect_root()` 当前 `(p/"server").exists()` 会在便携包
    `LabSentinel/app` 处提前命中（`app/server`），永远到不了含 `runtime/python.exe`/`tools` 的包根，
    `resolve_ffmpeg()` 仍找不到 `LabSentinel/tools/ffmpeg.exe`。修复：**先**找 `runtime/python.exe`
    的父目录为包根，找不到再找 `server/app/main.py`（源码根），最后才 fallback；补
    `test_detect_root_portable`（fake 包树）与 `test_detect_root_source` 两个用例，并在解压 zip 后
    实跑 `/api/cameras/usb-devices` 确认 `ffmpeg` 字段指向包内扁平 exe。

### 75.2 便携包缺少备份/诊断脚本

- 现状：`scripts/build_portable.py` 只复制 `portable_launcher.py`、`install_portable.py`、`seed_demo.py`、`doctor.py`；
  但 `docs/59` 与 `docs/运维手册.md` 让用户在便携包里运行 `scripts\backup_data.py`、`scripts\collect_diagnostics.py`。
- 要求：构建时复制 `backup_data.py`、`collect_diagnostics.py` 到 `DIST/scripts/`；保留 doctor/seed/launcher/installer。
- 验收：解压新 zip 后 `scripts\backup_data.py`/`collect_diagnostics.py` 存在，且
  `runtime\python.exe scripts\backup_data.py --help`、`... collect_diagnostics.py --help` 可运行。

### 75.3 collect_diagnostics 便携路径错误

- 现状：`ROOT = Path(__file__).resolve().parent.parent`；默认 data dir = `ROOT/server/data`，
  但便携包为 `ROOT/app/server`，数据在 `%LOCALAPPDATA%\LabSentinel\data`；import `app.config` 也用 `ROOT/server`。
- 要求：
  - 探测 server 目录：`ROOT/runtime/python.exe` 存在 → `ROOT/app/server`；否则 `ROOT/server`；
  - `--data-dir` 默认：便携 → `%LOCALAPPDATA%/LabSentinel/data`；源码 → `ROOT/server/data`；
  - `sys.path` 指向探测出的 server 目录；
  - 明确注释/帮助文本；源码/便携双布局测试。

### 75.4 诊断包泄露 SMTP 密码（安全）

- 现状：`collect_diagnostics._SENSITIVE = re.compile(r"TOKEN|PASSWORD|SECRET|KEY")`；
  `smtp_pass` 不含这些词 → 配置文本会输出明文密码。`redact_value` 只处理 URL 凭证。
- 要求：
  - 敏感键模式增加 `PASS`、`CREDENTIAL`、`AUTH`（用 `(?:^|_)` 或 `\b` 避免误伤 `bypass` 等），
    或显式列出 `smtp_pass`；
  - 测试：`smtp_pass=supersecret`、`LS_SMTP_PASS`、`webhook_secret`、`admin_token`、`qmsg_key` 全部输出 `***`；
    普通 `smtp_host`/`smtp_user` 不误脱敏；
  - 真实生成诊断包后 `grep` 不到测试密码。

### 75.5 release_check 误报自身测试夹具

- 现状：`server/tests/test_release_check.py` 写入假 key（`sk-abc…`），仓库终检报
  `FAIL 敏感信息 1 处`，与 74 报告“FAIL 0”不一致；且该文件被扫描。
- 要求（二选一，优先 A）：
  - A. 扫描器 allowlist 增加 `server/tests/test_release_check.py`（注释说明是测试夹具，非真实密钥）；
  - B. 测试夹具改用不匹配密钥模式的安全假值（如 `FAKE-KEY-NOT-REAL`），并保留 allowlist 机制测试。
- 验收：`python scripts/release_check.py` 对真实仓库 **FAIL 0**、退出码 0；测试仍覆盖“真密钥→FAIL”分支。

### 75.6 小修：middleware 层 500 缺 X-Request-ID 头

- `main.py` 的 `request_id_middleware` catch-all 返回 JSONResponse 时补 `headers={"X-Request-ID": rid}`；
  测试断言该响应头存在。

## 重建与复验（必须全做）

1. `cd web && npm run build`（确认 dist 最新）。
2. `python scripts/build_portable.py --zip`（用项目 `.venv`；产出 `dist/LabSentinel-portable-20261030.zip`）。
3. `python scripts/release_manifest.py --zip <新 zip>` + `--verify`；确认必需条目齐全、可选仅缺 `卸载.bat`。
4. `python scripts/smoke_portable.py --zip <新 zip>` → 8/8 PASS。
5. 解压新 zip 到临时目录：
   - `runtime\python.exe scripts\doctor.py --root <解压根>` → FAIL 0；
   - `scripts\backup_data.py --help`、`scripts\collect_diagnostics.py --help` 可运行；
   - 用隔离 `LOCALAPPDATA` 启动包（8181），调用 `/api/cameras/usb-devices`，确认能返回本机设备
     （本机应有 HD Webcam/OBS；若设备被占用，至少确认 error 不是“找不到 ffmpeg”）；
   - 生成一次诊断包（指向隔离 data），grep 不到任何真实 token/密码。
6. 更新 `docs/发布核对报告.md` 的便携包数字（新 zip 名/大小/sha256/条目）为**本次实测**；
   `docs/59` 若路径/命令有变化同步。
7. `pytest tests/ -q` 全绿（基线 165 + 新增）；`release_check.py` FAIL 0。

## 约束

- 只改上述文件；不要重构 57–74 其他功能；不引入依赖。
- 不改 `web/src/theme.css/style.css/App.vue/main.js`。
- 重建/冒烟会占端口（8180/8181）与 CPU，逐项做，不并发。
- 不推送、不打 tag、不发布。

## 执行报告

（待填：修复项 / 新 zip 数字 / smoke/doctor/diagnostics 证据 / pytest / release_check / 遗留）
