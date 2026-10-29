# 任务 71 · 安装器静默/自定义目录/可测试性

> 背景：`install_portable.py` 已有 `--install-dir` 参数，但 `do_install()` 里**硬编码**了
> `%LOCALAPPDATA%\Programs\LabSentinel`，参数实际不生效；也没有静默安装/无快捷方式/试运行模式，
> 企业批量部署和自动化验收都不方便。本任务修正参数语义并补齐可测试性。

## 目标

### 1. 参数行为（`scripts/install_portable.py`）

- `--install-dir` **真正生效**（install 与 uninstall 都按它；默认值保持不变）；
- `--silent`：不打印交互提示、不等待；退出码明确（0 成功，2 参数/环境错误）；
- `--no-shortcuts`：跳过桌面/开始菜单快捷方式（只装程序 + 可选自启）；
- `--no-autostart`：明确关闭自启（与 `--autostart` 互斥，后者优先或报错——选一种并测试）；
- `--dry-run`：只打印将执行的动作（目标目录、快捷方式、自启、数据目录），**不写任何文件/注册表**；
- `--desktop-dir` / `--start-menu-dir`（或环境变量 `LS_INSTALL_DESKTOP` /
  `LS_INSTALL_START_MENU`）：覆盖快捷方式位置，供隔离测试；默认保持现状；
- 安装完成后打印：版本、安装目录、启动方式、数据目录、卸载方式（silent 下也打印到 stdout）。

### 2. 安全/边界

- 安装目录必须是绝对路径；拒绝 `C:\Windows`、盘符根、用户主目录根等危险目标（白名单式提示 + exit 2）；
- 覆盖升级：保留数据目录（现有行为），程序文件覆盖；若目标目录不是 LabSentinel 安装（缺
  `runtime/python.exe` 且非空）→ 拒绝覆盖，除非 `--force`；
- 卸载：仍默认保留数据；`--purge-data` 才删；不改变现有延迟自删机制。

### 3. 隔离测试 `server/tests/test_installer_options.py`

- 用临时目录构造假便携包（`runtime/python.exe` 空文件、`scripts/portable_launcher.py`、
  `app/VERSION`）；`LOCALAPPDATA`/`APPDATA`/桌面全部指向临时目录；
- `--dry-run`：目标目录不存在、无快捷方式、无注册表调用（monkeypatch `subprocess.run` 记录）；
- `--install-dir`：安装到指定临时目录（`--no-shortcuts --no-autostart`），文件存在、版本文件正确；
- 覆盖升级：先装一次 → 在数据目录插一个文件 → 再装 → 数据文件仍在；
- 危险目录：`--install-dir C:\Windows` exit 2；非空非安装目录且无 `--force` exit 2；
- `--no-shortcuts`：不调用 PowerShell 快捷方式函数（monkeypatch 断言未调用）；
- 测试**绝不**写真实 HKCU/桌面；`set_autostart` 用 monkeypatch 或 `--no-autostart`。

### 4. 文档

- `docs/59-portable-install.md` 增加「静默安装/自定义目录/批量部署」命令表；
- `docs/部署FAQ.md` 增加「安装到 D 盘 / 公司批量部署 / 覆盖升级会丢数据吗」。

## 约束

- 不改变默认安装路径与默认数据路径（兼容现有用户）；
- 不引入依赖；Windows 路径/中文路径正确；
- 真实注册表/桌面只在手工验收时可选，自动化测试必须隔离；
- 不改 `LabSentinel.exe`/启动器逻辑。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；
- 临时目录实跑：默认安装、自定义目录、dry-run、覆盖升级四项均符合预期，报告附目录树/输出；
- 真实机器上不做安装（避免污染），只做 `--dry-run` 实跑；
- `--help` 列出全部新参数。

## 执行报告

（待填：改动文件 / 隔离安装证据 / 测试 / 遗留）
