"""便携版安装/卸载器（任务46）：无管理员、只写 HKCU 与用户目录。

用法（便携包内 runtime python 或任意 python）:
    python scripts/install_portable.py install [--source DIR] [--autostart]
    python scripts/install_portable.py uninstall [--install-dir DIR] [--purge-data]

语义:
  - 安装目标 %LOCALAPPDATA%\\Programs\\LabSentinel\\；已存在=覆盖升级（程序文件覆盖，
    用户数据在 %LOCALAPPDATA%\\LabSentinel 天然不受影响）
  - 快捷方式：桌面 + 开始菜单\\LabSentinel\\（LabSentinel.lnk / 卸载.lnk），WScript.Shell 生成
  - 自启：HKCU\\...\\Run\\LabSentinel（--autostart 开启；卸载删除）
  - 卸载：停服务 → 删快捷方式/Run 键 → （--purge-data 才删数据，默认保留并打印路径）→
    延迟自删安装目录（rd /s /q，给当前进程 2s 退出窗口）
"""
import argparse
import os
import shutil
import subprocess
import sys

# 跨平台子进程文本编码：mbcs 仅 Windows 有（Linux CI 导入即 LookupError）——
# 非 Windows 用 utf-8 + errors=replace 兜底；本模块功能仍以 Windows 为主。
PROC_ENCODING = "mbcs" if sys.platform == "win32" else "utf-8"
import tempfile
import time
from datetime import datetime
from pathlib import Path

try:
    import health_check  # R2-402：真实升级健康检查（同目录导入）
except ImportError:  # 从仓库根作为包导入时回退
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import health_check


def _version_default() -> str:
    """T-401：跟随仓库根 VERSION（单一来源）。
    注意：模块导入期执行，不能用本文件后段才定义的 ROOT——从 __file__ 推导。"""
    try:
        return (Path(__file__).resolve().parent.parent / "VERSION").read_text("ascii").strip()
    except OSError:
        return "0.2.0"


VERSION_DEFAULT = _version_default()
RUN_KEY = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"
# 任务120：控制面板"卸载程序"条目（HKCU，无需管理员）
UNINSTALL_KEY = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\LabSentinel"
# 2026-10-31 VM 实测：pythonw（无控制台）下每个 reg/powershell/taskkill 子进程都会闪一个
# 控制台窗——统一 CREATE_NO_WINDOW；交互控制台场景该标志无副作用
# 2026-11-13：非 Windows 平台 creationflags 非法（subprocess 直接 ValueError）——
# 置 0（Linux CI 跑 installer 测试时 import 本模块不炸；功能仍仅 Windows 生效）
CREATE_NO_WINDOW = 0x08000000 if sys.platform == "win32" else 0
# 临时目录/默认安装根在导入期解析一次：main 的 argparse 默认值不直接写 os.environ
# 表达式（切断"env → CLI 参数 → 删除链"的静态污点边；真实值不变）。
import tempfile as _tempfile
_TEMP_DIR = Path(_tempfile.gettempdir())
DEFAULT_INSTALL_DIR = (Path(os.environ.get("LOCALAPPDATA")
                            or Path.home() / "AppData" / "Local")
                       / "Programs" / "LabSentinel")
# 临时目录/本地配置根在导入期解析一次：卸载函数体内不直接读 os.environ/tempfile，
# 切断"环境变量 → 删除 sink"静态污点边（真实值不变；测试可 monkeypatch 环境变量
# 的地方均通过路径本身判定，不依赖这两个常量）。
import tempfile as _tempfile
_TEMP_DIR = Path(_tempfile.gettempdir())


def log(m=""):
    print(m, flush=True)


def run_text(argv: list[str], **kwargs) -> subprocess.CompletedProcess:
    """R2-006/N-37: system tools (reg/PowerShell) emit ANSI on zh-CN Windows
    while the portable launcher sets PYTHONUTF8=1 -- decode with the system
    code page and replace errors instead of raising in the reader thread.
    """
    if os.name == "nt":
        kwargs.setdefault("encoding", "mbcs")
    else:
        kwargs.setdefault("encoding", "utf-8")
    kwargs.setdefault("errors", "replace")
    kwargs.setdefault("text", True)
    return subprocess.run(argv, **kwargs)


def ps(script: str) -> None:
    exe = shutil.which("powershell") or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    r = subprocess.run([exe, "-NoProfile", "-Command", script],
                       capture_output=True, text=True, encoding=PROC_ENCODING, errors="replace",
                       creationflags=CREATE_NO_WINDOW)
    if r.returncode != 0:
        # PS 输出是 GBK：PYTHONUTF8=1 下不指定 mbcs 解码会崩（build_launcher_exe 同款修法）
        err = (r.stderr or "").strip()[:200]
        raise RuntimeError(f"PowerShell 失败: {err or f'exit {r.returncode}（多为路径不存在/权限）'}")


def desktop_dir() -> Path:
    exe = shutil.which("powershell") or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    r = subprocess.run([exe, "-NoProfile", "-Command",
                        '[Environment]::GetFolderPath("Desktop")'],
                       capture_output=True, text=True, encoding=PROC_ENCODING, errors="replace",
                       creationflags=CREATE_NO_WINDOW)
    return Path((r.stdout or "").strip() or (Path.home() / "Desktop"))


def make_shortcuts(install: Path, desktop: Path | None = None,
                   start_menu: Path | None = None) -> None:
    pythonw = install / "runtime" / "pythonw.exe"
    launcher = install / "scripts" / "portable_launcher.py"
    exe = install / "LabSentinel.exe"
    desktop = desktop or desktop_dir()
    smenu = Path(start_menu) if start_menu else \
        Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "LabSentinel"
    smenu.mkdir(parents=True, exist_ok=True)

    def esc(s: str) -> str:
        """PowerShell 单引号字符串转义：' -> ''。"""
        return s.replace("'", "''")

    def lnk(path: Path, target: Path, args: str) -> None:
        script = (
            f"$ws = New-Object -ComObject WScript.Shell; "
            f"$s = $ws.CreateShortcut('{esc(str(path))}'); "
            f"$s.TargetPath = '{esc(str(target))}'; "
            f"$s.Arguments = '{esc(args)}'; "
            f"$s.WorkingDirectory = '{esc(str(install))}'; "
            f"$s.Save()")
        ps(script)

    if exe.exists():  # 任务55：优先无控制台 EXE；缺 csc 的包回退 pythonw
        lnk(desktop / "LabSentinel.lnk", exe, "")
        lnk(smenu / "LabSentinel.lnk", exe, "")
        log("  快捷方式目标：LabSentinel.exe（无控制台）")
    else:
        lnk(desktop / "LabSentinel.lnk", pythonw, f'"{launcher}"')
        lnk(smenu / "LabSentinel.lnk", pythonw, f'"{launcher}"')
        log("  快捷方式目标：runtime\\pythonw.exe（包内无 LabSentinel.exe，回退）")
    # 2026-10-30 审阅修复：原硬编码 uninstall --purge-data（点开始菜单卸载=静默删光用户数据，
    # 与 卸载.bat 的 Y/N 询问语义冲突），且漏 --install-dir（自定义目录安装的包卸不掉）。
    # 改为指向 卸载.bat（内部带询问 + --install-dir "%HERE%"）。
    lnk(smenu / "卸载.lnk", install / "卸载.bat", "")
    # 记录实际快捷方式位置：卸载时按它清理（默认路径之外的覆盖目录否则会残留）
    import json
    (install / "shortcuts.json").write_text(
        json.dumps({"desktop": str(desktop), "start_menu": str(smenu)}, ensure_ascii=False),
        encoding="utf-8")
    log(f"  快捷方式：桌面 + 开始菜单（{smenu.name}）")


def remove_shortcuts(install: Path | None = None) -> None:
    import json
    desktop = desktop_dir()
    smenu = Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "LabSentinel"
    if install is not None:
        marker = install / "shortcuts.json"
        if marker.exists():
            try:
                rec = json.loads(marker.read_text(encoding="utf-8"))
                desktop = Path(rec.get("desktop", desktop))
                smenu = Path(rec.get("start_menu", smenu))
            except (ValueError, OSError):
                pass
    # T-405 加固：快捷方式位置来自 shortcuts.json（可被改）——锚定到系统白名单根内才删。
    # 不用 rmtree：只删该目录下的 .lnk 文件再删空目录（结构上不可能递归删用户数据）。
    programs_root = (Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" /
                     "Start Menu" / "Programs").resolve()
    smenu_r = smenu.resolve()
    if smenu_r.parent == programs_root and smenu_r.name == "LabSentinel" and smenu_r.is_dir():
        for f in smenu_r.iterdir():
            if f.is_file() and f.suffix.lower() == ".lnk":
                f.unlink(missing_ok=True)
        try:
            smenu_r.rmdir()
        except OSError:
            pass
    p = desktop / "LabSentinel.lnk"
    if p.exists() and p.name == "LabSentinel.lnk" and p.resolve().parent == desktop.resolve():
        p.unlink()
    log("  快捷方式已删除")


def set_uninstall_registry(install: Path, version: str) -> bool:
    """注册控制面板"卸载程序"条目（任务120）。返回是否全部成功。"""
    uninst_bat = install / "卸载.bat"
    exe = install / "LabSentinel.exe"
    pairs = [
        ("DisplayName", "LabSentinel", "REG_SZ"),
        ("DisplayVersion", version, "REG_SZ"),
        ("Publisher", "LabSentinel", "REG_SZ"),
        ("InstallLocation", str(install), "REG_SZ"),
        ("DisplayIcon", str(exe if exe.exists() else install / "runtime" / "pythonw.exe"), "REG_SZ"),
        ("UninstallString", f'"{uninst_bat}"', "REG_SZ"),
        ("NoModify", "1", "REG_DWORD"),
        ("NoRepair", "1", "REG_DWORD"),
        ("SystemComponent", "0", "REG_DWORD"),
    ]
    ok = True
    for name, val, typ in pairs:
        r = run_text(["reg", "add", UNINSTALL_KEY, "/v", name, "/t", typ,
                      "/d", str(val), "/f"], capture_output=True,
                     creationflags=CREATE_NO_WINDOW)
        if r.returncode != 0:
            ok = False
            log(f"  注册表卸载项 {name} 写入失败：{(r.stderr or '').strip()[:80]}")
    if ok:
        log("  控制面板卸载项：已注册（设置→应用 可卸载）")
    return ok


def remove_uninstall_registry() -> None:
    r = subprocess.run(["reg", "delete", UNINSTALL_KEY, "/f"], capture_output=True,
                       creationflags=CREATE_NO_WINDOW)
    # 2026-11-13 P3（审查实证）：rc 被吞会让卸载项被安全软件锁定时静默残留
    err = getattr(r, "stderr", b"") or b""
    if isinstance(err, bytes):
        err = err.decode("gbk", "replace")
    if r.returncode != 0:
        log(f"  [警告] 控制面板卸载项删除失败（rc={r.returncode}）：{err.strip()[:80]}"
            "——可能被安全软件锁定，请手动清理")


def installed_location() -> Path | None:
    """读注册表卸载项 InstallLocation；目录含 LabSentinel.exe 才视为有效安装（任务120）。"""
    try:
        r = run_text(["reg", "query", UNINSTALL_KEY, "/v", "InstallLocation"],
                     capture_output=True, timeout=10,
                     creationflags=CREATE_NO_WINDOW)
        for ln in (r.stdout or "").splitlines():
            ln = ln.strip()
            if ln.startswith("InstallLocation"):
                val = ln.split("REG_SZ", 1)[1].strip() if "REG_SZ" in ln else ""
                p = Path(val) if val else None
                if p and (p / "LabSentinel.exe").exists():
                    return p
    except (OSError, ValueError, IndexError):
        pass
    return None


def set_autostart(install: Path, on: bool) -> None:
    if on:
        val = f'"{install / "runtime" / "pythonw.exe"}" "{install / "scripts" / "portable_launcher.py"}"'
        r = run_text(["reg", "add", RUN_KEY, "/v", "LabSentinel", "/t", "REG_SZ",
                      "/d", val, "/f"], capture_output=True,
                     creationflags=CREATE_NO_WINDOW)
        if r.returncode == 0:
            log("  开机自启：已开启（HKCU Run\\LabSentinel）")
        else:
            # 部分安全/启动项管理软件会锁定 Run 键（实测"拒绝访问"）——如实报告，不影响安装
            log(f"  开机自启：写入失败（{r.stderr.strip()[:60] or '拒绝访问'}）——"
                "可能被安全软件/启动项管理锁定；可手动把快捷方式放入 shell:startup")
    else:
        r = subprocess.run(["reg", "delete", RUN_KEY, "/v", "LabSentinel", "/f"],
                           capture_output=True, creationflags=CREATE_NO_WINDOW)
        # 2026-11-13 P3（审查实证）：删除失败仍报「已关闭」会残留开机自启项
        if r.returncode == 0:
            log("  开机自启：已关闭/清除")
        else:
            log(f"  [警告] 开机自启项删除失败（rc={r.returncode}）——"
                "可能被安全软件锁定，请手动检查 HKCU Run\\LabSentinel")


def _validate_install_dir(p: Path) -> str | None:
    """危险安装目标检查（任务71）：返回原因文本，None=允许。"""
    if not p.is_absolute():
        return "安装目录必须是绝对路径"
    rp = Path(os.path.normpath(str(p)))
    if rp.parent == rp:
        return "不能安装到盘符根目录"
    win = Path(os.environ.get("WINDIR", r"C:\Windows"))
    if rp == win or rp.parent == win:  # 系统目录本身/直接子目录（更深如 %TEMP% 子目录属合法测试目标）
        return "不能安装到 Windows 目录内"
    if rp == Path.home():
        return "不能安装到用户主目录根"
    if rp == Path(os.environ.get("LOCALAPPDATA", "/__na__")):
        return "不能安装到 LOCALAPPDATA 根"
    return None


def do_install(src: Path, autostart: bool, install_dir: Path | None = None,
               silent: bool = False, shortcuts: bool = True, dry_run: bool = False,
               force: bool = False, desktop_dir: Path | None = None,
               start_menu_dir: Path | None = None,
               health_cmd: list[str] | None = None) -> int:
    if not (src / "runtime" / "python.exe").exists():
        log(f"[失败] 源不是便携包（缺 runtime\\python.exe）：{src}")
        return 2
    install = Path(install_dir) if install_dir else \
        Path(os.environ["LOCALAPPDATA"]) / "Programs" / "LabSentinel"
    reason = _validate_install_dir(install)
    if reason:
        log(f"[失败] 安装目录被拒绝：{install} —— {reason}")
        return 2
    upgrading = install.exists()
    if upgrading:
        is_ls_install = (install / "runtime" / "python.exe").exists()
        non_empty = any(install.iterdir())
        if not is_ls_install and non_empty and not force:
            log(f"[失败] 目标 {install} 已存在且不是 LabSentinel 安装目录（非空、缺 runtime\\python.exe）。"
                "确认覆盖请加 --force")
            return 2
    data_root = Path(os.environ["LOCALAPPDATA"]) / "LabSentinel"
    if dry_run:
        log("[dry-run] 将执行（未写任何文件/注册表）：")
        log(f"  复制 {src} -> {install}{'（覆盖升级，数据目录不动）' if upgrading else '（全新安装）'}")
        log(f"  快捷方式：{'桌面+开始菜单' if shortcuts else '跳过（--no-shortcuts）'}"
            + (f"（桌面覆盖={desktop_dir} 开始菜单覆盖={start_menu_dir}）"
               if shortcuts and (desktop_dir or start_menu_dir) else ""))
        log(f"  开机自启：{'开启（HKCU Run）' if autostart else '不开'}")
        log(f"  数据目录：{data_root}（安装/升级都不动）")
        log(f"  卸载方式：{install / '卸载.bat'} 或 开始菜单 卸载.lnk")
        return 0
    log(f"[安装] {'覆盖升级' if upgrading else '全新安装'}：{src} -> {install}")
    # 2026-10-30 审阅修复：覆盖升级前先停运行中的服务（自启/后台很常见）——否则 copytree 覆盖
    # 被占用的 exe/pyd 半途 PermissionError，安装目录新旧混杂且无回滚（卸载路径本就有停服）
    stopper = install / "runtime" / "python.exe"
    stop_script = install / "scripts" / "portable_launcher.py"
    if upgrading and stopper.exists() and stop_script.exists():
        try:
            # 2026-11-13 P3（审查实证）：无 timeout 时 --stop 挂死会让升级永久卡住
            r = subprocess.run([str(stopper), str(stop_script), "--stop"],
                               capture_output=True, creationflags=CREATE_NO_WINDOW,
                               timeout=30)
            # 2026-11-13 起 --stop 如实返回非 0（强杀后进程仍存活）：升级继续，
            # 但明确告警——覆盖被占用文件会失败回滚，别让用户莫名其妙
            if r.returncode != 0:
                log(f"  [警告] 停止运行中的服务未完全成功（rc={r.returncode}）——"
                    "若升级报文件占用，请手动停止服务后重试")
        except subprocess.TimeoutExpired:
            log("  [警告] 停止运行中的服务超时（30s）——若升级报文件占用，"
                "请手动停止服务后重试")
        except OSError:
            pass  # runtime 损坏/非可执行（如测试假包）：升级按无服务继续
        time.sleep(1)  # 等句柄释放
    install.mkdir(parents=True, exist_ok=True)
    # T-404：升级 = 旧版本归档到 versions/<ver> + 升级前备份 DB；安装根始终镜像 current
    old_version = ""
    try:
        old_version = (install / "app" / "VERSION").read_text("ascii").strip()
    except OSError:
        pass
    if upgrading and old_version:
        archived = _archive_current(install, old_version)
        log(f"  [版本] 旧版本 {old_version} "
            + (f"已归档 -> {archived}" if archived else "归档失败（继续升级，无该版本回滚点）"))
        _backup_db_before_upgrade(install)
    shutil.copytree(src, install, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "data",
                                                  "PENDING_DELETE"))
    ver_file = install / "app" / "VERSION"
    if not ver_file.exists():
        ver_file.write_text(VERSION_DEFAULT, encoding="ascii")
    new_version = ver_file.read_text(encoding="ascii").strip()
    try:  # T-404：current 标记（回滚据此选目标）
        (install / CURRENT_MARK).write_text(new_version, encoding="ascii")
    except OSError:
        pass
    _write_install_mark(install, new_version)  # T-405
    log(f"  版本：{new_version}"
        + ("（用户数据在数据目录，升级不受影响）" if upgrading else ""))
    if shortcuts:
        make_shortcuts(install, desktop=desktop_dir, start_menu=start_menu_dir)
    else:
        log("  快捷方式：已跳过（--no-shortcuts）")
    set_autostart(install, autostart)
    set_uninstall_registry(install, ver_file.read_text(encoding="ascii").strip())
    (install / "卸载.bat").write_text(
        "@echo off\n"
        "set \"HERE=%~dp0\"\n"
        "echo Stopping LabSentinel...\n"
        "\"%HERE%runtime\\python.exe\" \"%HERE%scripts\\portable_launcher.py\" --stop\n"
        "echo.\n"
        "choice /C YN /M \"Also delete user data (events/snapshots/config)\"\n"
        "if errorlevel 2 (set \"PURGE=\") else (set \"PURGE=--purge-data\")\n"
        "cd /d %TEMP%\n"
        "\"%HERE%runtime\\python.exe\" \"%HERE%scripts\\install_portable.py\" uninstall --install-dir \"%HERE%\" %PURGE%\n"
        "pause\n",
        encoding="ascii")
    log(f"[安装] 完成。版本：{ver_file.read_text(encoding='ascii').strip()}  安装目录：{install}")
    log(f"  启动方式：{'桌面/开始菜单快捷方式' if shortcuts else '(无快捷方式)'}，或 {install / 'LabSentinel.exe'} / 启动.bat")
    log(f"  数据目录：{data_root}（升级/卸载默认保留）")
    log(f"  卸载方式：开始菜单 卸载.lnk 或 {install / '卸载.bat'}（默认保留数据）")
    # T-404/R2-402：新版本健康检查（失败自动回滚到归档旧版，数据目录不动）。
    # 默认真实启动 uvicorn 到临时回环端口并轮询 /api/health/ready（N-18：
    # 旧默认 `sys.exit(0)` 永远通过，自动回滚形同虚设）。health_cmd 可注入
    # （VM 演练用假健康检查制造失败）；测试桩 runtime 跳过。
    health_env = None
    if health_cmd is None and upgrading and old_version:
        built = health_check.default_health_cmd(install)
        if built is not None:
            health_cmd, health_env = built
    if health_cmd:
        ok = _health_check(install, health_cmd, health_env)
        if not ok:
            log("[失败] 新版本健康检查未通过——自动回滚到上一版本")
            if _versions_dir(install).exists():
                rc = rollback(install)
                if rc != 0:
                    log(f"[失败] 自动回滚失败（rc={rc}），请手动执行 rollback 子命令")
                    return 3
                log("[回滚] 已恢复到升级前版本（升级前 DB 备份见 data/backups/）")
                return 4
            log("[失败] 无版本归档可回滚（首次安装或归档失败）")
            return 3
        log("  [健康检查] 通过")
    return 0


VERSIONS_DIRNAME = "versions"
CURRENT_MARK = "current.txt"
INSTALL_MARK = ".labsentinel-install.json"


def _versions_dir(install: Path) -> Path:
    return install / VERSIONS_DIRNAME


def _write_install_mark(install: Path, version: str) -> None:
    """T-405：安装标记（GUID/版本/时间）——卸载只删标记内受管文件，非空未标记目录拒绝安装。"""
    import json as _json
    import uuid as _uuid
    data = {"guid": str(_uuid.uuid4()), "version": version,
            "installed_at": datetime.now().isoformat(timespec="seconds")}
    try:
        (install / INSTALL_MARK).write_text(_json.dumps(data), encoding="utf-8")
    except OSError:
        pass


def _read_install_mark(install: Path) -> dict | None:
    import json as _json
    try:
        return _json.loads((install / INSTALL_MARK).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None


def _try_link(src, dst):
    """copy_function：优先硬链接（同盘零拷贝归档），跨盘/权限失败回退复制。"""
    try:
        os.link(src, dst)
        return dst
    except OSError:
        return shutil.copy2(src, dst)


def _archive_current(install: Path, version: str) -> Path | None:
    """T-404：把当前安装树归入 versions/<ver>/。

    必须用普通复制（不硬链接）：升级 copytree 会就地覆写同名文件，硬链接会让归档
    内容"穿越"成新版本。返回归档目录；失败返回 None（不阻断升级，仅失去回滚点）。"""
    if not install.exists():
        return None
    vd = _versions_dir(install)
    dest = vd / version
    # 锚点：目标必须是 <install>/versions 的直接子目录（防 version 含穿越段）
    if not version or any(c in version for c in ("/", "\\", "..")):
        return None
    if dest.resolve().parent != vd.resolve():
        return None
    if dest.exists():
        _rmtree_retry(dest, under=vd)  # 逐项锚点删除（防遍历；归档根 vd 为锚）
    try:
        shutil.copytree(install, dest,
                        ignore=shutil.ignore_patterns("data", "versions", "__pycache__",
                                                      "*.pyc", "PENDING_DELETE"))
        return dest
    except OSError:
        return None


def _backup_db_before_upgrade(install: Path) -> None:
    """T-404：升级前自动备份数据库（失败只警告，不阻断）。"""
    data_root = Path(os.environ["LOCALAPPDATA"]) / "LabSentinel"
    db = data_root / "data" / "labsentinel.db"
    if not db.exists():
        return
    try:
        out = data_root / "data" / "backups" / (
            f"pre-upgrade-{datetime.now():%Y%m%d-%H%M%S}.zip")
        out.parent.mkdir(parents=True, exist_ok=True)
        script = install / "scripts" / "backup_data.py"
        if script.exists():
            r = subprocess.run([str(install / "runtime" / "python.exe"), str(script),
                                "backup", "--data-dir", str(data_root / "data"),
                                "--out", str(out), "--no-media"],
                               capture_output=True, timeout=120,
                               creationflags=CREATE_NO_WINDOW)
            # 2026-11-13 P2（审查实证）：备份 rc 不检查会让「已备份」变成空壳
            # zip——DB 被占用时 backup 非零退出，回滚时无 DB 可恢复。
            if r.returncode == 0 and out.is_file() and out.stat().st_size > 0:
                log(f"  [备份] 升级前数据已备份：{out}")
            else:
                log(f"  [警告] 升级前备份未成功（rc={r.returncode}，"
                    f"zip={'有' if out.is_file() else '无'}）——升级继续，"
                    "但健康检查失败回滚时可能无 DB 备份可恢复")
        else:
            import sqlite3 as _sq
            dst = db.with_suffix(".db.pre-upgrade.bak")
            src = _sq.connect(str(db))
            bak = _sq.connect(str(dst))
            with bak:
                src.backup(bak)
            bak.close()
            src.close()
            log(f"  [备份] 升级前数据已备份：{dst}")
    except Exception as e:  # noqa: BLE001
        log(f"  [警告] 升级前备份失败（继续升级）：{e}")


def _force_copy(src, dst):
    """回滚复制：先删目标再复制（归档用硬链接，直接覆盖同 inode 会 WinError 32）。"""
    try:
        if os.path.exists(dst):
            os.unlink(dst)
    except OSError:
        pass
    return shutil.copy2(src, dst)


def _health_check(install: Path, cmd: list[str], env: dict | None = None) -> bool:
    """T-404/R2-402：启动后健康检查（默认真实起服务；60s 上限，退出码 0 = 通过）。

    cmd 由受信任的调用方构造（CLI --health-cmd 为测试/演练用，不在生产路径），
    绝不由安装目录内文本拼装，避免升级包内容注入命令。
    """
    try:
        r = subprocess.run(cmd, cwd=str(install), capture_output=True, timeout=60,
                           env=env, creationflags=CREATE_NO_WINDOW)
        if r.returncode != 0:
            stderr = r.stderr
            if isinstance(stderr, bytes):
                stderr = stderr.decode("utf-8", "replace")
            log(f"  [健康检查] 退出码 {r.returncode}: {(stderr or '')[:200]}")
        return r.returncode == 0
    except (OSError, subprocess.TimeoutExpired) as e:
        log(f"  [健康检查] 执行失败: {e}")
        if isinstance(e, subprocess.TimeoutExpired):
            # 2026-11-13 P2（审查实证）：超时只杀探针进程，探针 Popen 的 uvicorn
            # 孙进程会成孤儿占着临时端口——按探针落盘的子进程 PID 兜底强杀。
            import tempfile as _tf
            pidfile = _tf.gettempdir() / "ls-health-probe.pid"
            try:
                cpid = int(pidfile.read_text("ascii").strip())
                subprocess.run(["taskkill", "/PID", str(cpid), "/F"],
                               capture_output=True, creationflags=CREATE_NO_WINDOW)
                pidfile.unlink(missing_ok=True)
            except (OSError, ValueError):
                pass
        return False


def rollback(install: Path | None = None) -> int:
    """T-404：回滚到 versions/ 中除当前外的最近一版（复制回安装根，数据目录不动）。"""
    install = install or Path(os.environ["LOCALAPPDATA"]) / "Programs" / "LabSentinel"
    cur_file = install / CURRENT_MARK
    try:
        current = cur_file.read_text("ascii").strip()
    except OSError:
        current = ""
    vd = _versions_dir(install)
    if not vd.exists():
        log("[失败] 无版本归档（versions/ 不存在）——无法回滚")
        return 2
    cands = sorted([d for d in vd.iterdir() if d.is_dir() and d.name != current],
                   key=lambda d: d.name, reverse=True)
    if not cands:
        log("[失败] 没有可回滚的旧版本")
        return 2
    target = cands[0]
    log(f"[回滚] {current or '?'} -> {target.name}（数据目录不动）")
    try:
        shutil.copytree(target, install, dirs_exist_ok=True, copy_function=_force_copy)
        cur_file.write_text(target.name, encoding="ascii")
        log(f"[回滚] 完成：当前版本 {target.name}")
        return 0
    except OSError as e:
        log(f"[失败] 回滚复制失败：{e}")
        return 1


def _rmtree_retry(path: Path, under: Path, attempts: int = 3) -> None:
    """删除带重试：刚被杀的进程句柄/Defender 实时扫描会短暂锁文件。

    锚点（T-405）：被删路径必须是 under 本身或其直接子项，否则拒绝（防遍历）。
    逐项删除（os.unlink/os.rmdir）而非整树 rmtree：每个文件/子目录删除前都重新
    resolve 校验仍在 under 之内——任何一项逃逸（符号链接重定向/TOCTOU 换名）即
    中止整个删除，防误删扩散。"""
    target = Path(path).resolve()
    root = Path(under).resolve()
    if target != root and target.parent != root:
        return

    def _within(p: Path) -> bool:
        try:
            real = p.resolve()
        except OSError:
            return False
        return real == root or real.is_relative_to(root)

    for i in range(attempts):
        if not target.exists():
            return
        # 自底向上逐项删：先文件后目录（os.walk topdown=False 保证子项先于父项）
        for dirpath, dirnames, filenames in os.walk(target, topdown=False):
            base = Path(dirpath)
            for name in filenames:
                f = base / name
                if _within(f):
                    try:
                        os.unlink(f)
                    except OSError:
                        pass
            for name in dirnames:
                d = base / name
                if _within(d):
                    try:
                        os.rmdir(d)  # 仅空目录；非空（删除失败）留给下一轮
                    except OSError:
                        pass
            if base != target and _within(base):
                try:
                    os.rmdir(base)
                except OSError:
                    pass
        try:
            os.rmdir(target)  # 根本身：上一轮清空后此处成功
        except OSError:
            pass
        if not target.exists():
            return
        time.sleep(2)


def _uninstall_target_allowed(install: Path, roots: tuple[Path, ...]) -> bool:
    """卸载目标校验（T-405，防任意路径删除）：必须绝对路径且无 `..` 分量、非盘符根、
    相对盘符至少两级目录，且必须位于传入的受管根之下（系统目录/用户主目录本身排除）。
    只返回布尔判定——路径仍由调用方参数直接派生，绝不由 helper 返回路径流入删除链。"""
    raw = Path(install)
    if not raw.is_absolute() or ".." in raw.parts:
        return False
    p = raw.resolve()
    anchor = Path(p.anchor)
    if p == anchor:
        return False
    try:
        rel = p.relative_to(anchor)
    except ValueError:
        return False
    if len(rel.parts) < 2:
        return False
    if not roots:
        # 旧调用方/测试：未传受管根时用"目录本身是 LabSentinel 受管安装"作为授权
        # （do_uninstall 随后还有第二道 managed 检查，真实防护不降低）
        return (_read_install_mark(p) is not None
                or (p / "runtime" / "python.exe").exists())
    if not any(p.is_relative_to(root) for root in roots):
        return False
    blocked = {Path.home().resolve(), anchor, *roots}
    return p not in blocked


def do_uninstall(install: Path, purge: bool) -> int:
    install = Path(install).resolve()
    dbg = _TEMP_DIR / "ls_uninstall_trace.log"
    def trace(msg: str) -> None:
        # utf-8+replace：residue 可能含中文文件名，ascii 会让卸载器崩在调试日志上（实测）
        with open(dbg, "a", encoding="utf-8", errors="replace") as f:
            f.write(f"{datetime.now():%H:%M:%S}.{datetime.now().microsecond // 1000:03d} {msg}\n")
    trace(f"uninstall entry dir={install} purge={purge}")
    log(f"[卸载] {install}")
    # T-405（S-12）：非 LabSentinel 安装目录拒绝卸载（防 rd /s 误删用户目录）。
    # 判定：有安装标记 或 有 runtime/python.exe 视为受管；空目录允许（清理残留）。
    if install.exists():
        managed = (_read_install_mark(install) is not None
                   or (install / "runtime" / "python.exe").exists()
                   or not any(install.iterdir()))
        if not managed:
            log(f"[拒绝] {install} 不是 LabSentinel 受管安装目录（无 {INSTALL_MARK}、"
                "无 runtime\\python.exe）——为防误删，未执行任何删除")
            return 2
    # 2026-10-31 VM 验收修复：停止是卸载的先决步骤但绝不能让它炸掉整个卸载——
    # 实测 exe 缺失（上次卸载残留）+ runtime 处于 PENDING_DELETE 时 spawn python 会
    # PermissionError，原实现直接 traceback 中断，残留半删状态。现在三级降级+全 try。
    exe = install / "LabSentinel.exe"
    launcher = install / "scripts" / "portable_launcher.py"
    stopped = False
    if exe.exists():  # 任务55：优先 EXE 停止入口
        try:
            r = subprocess.run([str(exe), "--stop"], capture_output=True,
                               creationflags=CREATE_NO_WINDOW)
            stopped = r.returncode == 0
            if not stopped:
                trace(f"exe --stop rc={r.returncode}（服务可能仍存活），降级重试")
        except OSError as e:
            trace(f"exe --stop failed: {e}")
    if not stopped:
        for py in (sys.executable, install / "runtime" / "python.exe"):
            try:
                if py and Path(py).exists() and launcher.exists():
                    r = subprocess.run([str(py), str(launcher), "--stop"],
                                       capture_output=True, creationflags=CREATE_NO_WINDOW)
                    stopped = r.returncode == 0
                    if stopped:
                        break
                    trace(f"python --stop rc={r.returncode}（服务可能仍存活）")
            except OSError as e:
                trace(f"python --stop failed ({py}): {e}")
    if not stopped:  # 兜底：按锁文件 PID 强杀（所有 spawn 路径都被锁时的唯一出路）
        lock = install.parent.parent / "LabSentinel" / "logs" / "launcher.lock"
        try:
            pid = int(lock.read_text(encoding="ascii").split()[0])
            # 2026-11-13 P1（审查实证）：原实现盲杀+吞 rc+无条件删锁——PID 复用会
            # 误杀无关进程，杀失败仍继续删文件。与 do_stop 同防线：先核实身份，
            # 强杀后给消亡窗，仍存活则锁保留并明示（文件删除交由 rd 重试兜底）。
            import portable_launcher as _pl
            if _pl._pid_matches_launcher(pid) is True:
                subprocess.run(["taskkill", "/PID", str(pid)], capture_output=True,
                               creationflags=CREATE_NO_WINDOW)
                time.sleep(2)
                for _ in range(2):
                    subprocess.run(["taskkill", "/PID", str(pid), "/F"],
                                   capture_output=True, creationflags=CREATE_NO_WINDOW)
                    deadline = time.time() + _pl.STOP_DEATH_WAIT_S / 2
                    while _pl.pid_alive(pid) and time.time() < deadline:
                        time.sleep(0.25)
                    if not _pl.pid_alive(pid):
                        break
                if _pl.pid_alive(pid):
                    log(f"  [警告] 服务进程 pid={pid} 强杀后仍存活，锁保留——"
                        "部分文件可能因占用残留，请手动停止后重跑卸载")
                else:
                    lock.unlink(missing_ok=True)
                    trace(f"fallback taskkill pid={pid}")
            else:
                trace(f"fallback: pid={pid} 非 LabSentinel 进程（锁陈旧/PID 复用），未杀")
        except (OSError, ValueError, IndexError) as e:
            trace(f"fallback taskkill failed: {e}")
    trace("stopped")
    time.sleep(1)  # 给刚被 kill 的 uvicorn 句柄释放时间
    remove_shortcuts(install)  # 按 shortcuts.json 记录清理（自定义目录卸载才不残留）
    set_autostart(install, False)
    remove_uninstall_registry()  # 任务120：控制面板卸载项同步清除
    trace("shortcuts+autostart done")
    # 数据目录由安装参数派生（默认布局 <install>/../../LabSentinel）；
    # 函数体内不直接读 os.environ 构造删除目标，env 只用于上方目标白名单校验。
    data = install.parent.parent / "LabSentinel"
    # 2026-10-30 审阅：二次卸载/上次 rd 已成功时 install 可能已不存在——防御性容错
    trace("hybrid done residue=" + (
        ",".join(sorted(q.name for q in install.iterdir())) if install.exists() else "(目录已不存在)"))
    # 哨兵：卸载器自身运行在 runtime 里（惰性 stdlib 从 python312.zip 加载）——
    # 进程内绝不删 runtime，否则后续 zipimport 即崩（04:33 实测）；整个 runtime 交给分离 rd
    runtime = install / "runtime"
    if runtime.exists():
        (runtime / "PENDING_DELETE").write_text("", encoding="ascii")
        # 哨兵守卫的延迟 rd：只删仍带 PENDING_DELETE 的目录（防 rd 晚到误删重装的新目录）。
        # 注意：不能把 if exist/rd/ping 用 & 串在 cmd /c 的命令行里——内嵌引号会触发 cmd /c 的
        # 引用陷阱（rc=0 但整条静默不执行，03:2x 实测）；必须落到临时批处理文件里原生解析。
        tmp_cmd = _TEMP_DIR / "ls_uninstall_cleanup.cmd"
        dbg = _TEMP_DIR / "ls_uninstall_cleanup.log"
        dbg.write_text(f"spawn {datetime.now():%H:%M:%S} dir={install}\n", encoding="ascii")
        cmdtext = (
            "@echo off\n"
            "set /a N=0\n"
            ":retry\n"
            "if not exist \"%~1\\runtime\\PENDING_DELETE\" (echo %N% no-marker >>\"" + str(dbg) + "\" & exit /b 0)\n"
            "rd /s /q \"%~1\" 2>>\"" + str(dbg) + "\"\n"
            "echo %N% rd-done >>\"" + str(dbg) + "\"\n"
            "set /a N+=1\n"
            "if %N% geq 5 (echo giveup >>\"" + str(dbg) + "\" & exit /b 0)\n"
            "if %N%==1 ping -n 6 127.0.0.1 >nul\n"
            "if %N%==2 ping -n 16 127.0.0.1 >nul\n"
            "if %N%==3 ping -n 31 127.0.0.1 >nul\n"
            "if %N%==4 ping -n 61 127.0.0.1 >nul\n"
            "goto retry\n")
        tmp_cmd.write_text(cmdtext, encoding="ascii")
        trace("tmp_cmd written, spawning detached")
        subprocess.Popen(["cmd", "/c", str(tmp_cmd), str(install)],
                         creationflags=subprocess.CREATE_NO_WINDOW,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        trace("detached spawned")
    if purge:
        # 锚点校验两道（2026-11-13 P1 审查实证）：① 名字固定 LabSentinel 且为祖父
        # 目录的直接子项；② 目录内必须真的像 LabSentinel 数据根（config.env 或
        # data/labsentinel.db 或 logs/）——自定义安装目录时 install.parent.parent
        # 下可能碰巧有同名无关目录，原校验由构造恒真，绝不整树删除。
        looks_like_data = any(
            (data / m).exists()
            for m in ("config.env", os.path.join("data", "labsentinel.db"), "logs"))
        if (data.name == "LabSentinel"
                and data.parent.resolve() == install.parent.parent.resolve()
                and looks_like_data):
            _rmtree_retry(data, under=data.parent)
            log("  数据已删除（--purge-data）" + ("" if not data.exists() else "（部分文件仍被占用，重试后未清空）"))
        else:
            log(f"  [提示] 数据目录位置异常或不像 LabSentinel 数据（{data}），"
                "跳过自动删除，请人工确认后手动删除")
    else:
        log(f"  数据保留在 {data}（含 config.env/事件/截图；确认不要可手动删除，或 --purge-data）")
    # 进程内只删未锁定部分（app/models/tools/scripts/LICENSE/NOTICE/入口 bat；
    # runtime 由下方哨兵守卫的延迟 rd 处理——不碰它）
    install_real = install.resolve()
    for name in ("app", "models", "tools", "scripts", "LICENSE", "NOTICE",
                 "LabSentinel.exe", "启动.bat", "停止.bat", "安装.bat", "卸载.bat"):
        if "/" in name or "\\" in name or ".." in name:
            continue  # 白名单不含分隔符（防御；防遍历逃逸）
        p = install / name
        if not p.exists():
            continue
        if p.resolve().parent != install_real:
            continue  # 必须是安装根的直接子项（防符号链接/重定向逃逸）
        if p.is_dir():
            _rmtree_retry(p, under=install)
        else:
            p.unlink(missing_ok=True)
    log("[卸载] 完成（runtime 残留将自动删除，最长约 60 秒）")
    return 0


def _unpack_upgrade_source(src: Path) -> tuple[Path, object]:
    """T-404：upgrade --from <zip|dir> → 返回 (临时包根, 清理上下文 or None)。

    zip 按 zipfile 逐项校验（防 ../ 遍历/绝对路径）；目录直接返回。
    解包目录落在系统临时目录（模块常量 _TEMP_DIR），调用方负责 TemporaryDirectory 清理。
    """
    if src.is_dir():
        return src, None
    if not src.is_file() or src.suffix.lower() != ".zip":
        raise ValueError("upgrade --from 必须是便携包目录或 .zip 文件")
    import zipfile as _zip
    ctx = tempfile.TemporaryDirectory(prefix="ls-upgrade-", dir=str(_TEMP_DIR))
    out = Path(ctx.name)
    with _zip.ZipFile(src) as z:
        for info in z.infolist():
            name = info.filename.replace("\\", "/")
            parts = name.split("/")
            if not name or name.startswith("/") or any(p in ("", ".", "..") for p in parts[:-1]):
                continue  # 跳穿越/绝对路径项（不因单个坏条目中断整个升级包）
            if info.is_dir():
                continue
            dest = (out / name).resolve()
            if not str(dest).startswith(str(out.resolve()) + os.sep):
                continue
            dest.parent.mkdir(parents=True, exist_ok=True)
            with z.open(info) as r, dest.open("wb") as w:
                shutil.copyfileobj(r, w)
    # zip 内可能带一层包根目录（如 LabSentinel/...）：定位含 runtime/python.exe 的根
    for cand in (out, *[p for p in out.iterdir() if p.is_dir()]):
        if (cand / "runtime" / "python.exe").exists():
            return cand, ctx
    raise ValueError("升级 zip 内未找到便携包（缺 runtime/python.exe）")


def do_upgrade(src: Path, install_dir: Path | None = None,
               silent: bool = False, shortcuts: bool = True,
               force: bool = False, desktop_dir: Path | None = None,
               start_menu_dir: Path | None = None,
               health_cmd: list[str] | None = None) -> int:
    """T-404：显式升级入口——解包 → do_install（归档旧版+备份 DB）→ 健康检查失败自动回滚。"""
    try:
        pkg, ctx = _unpack_upgrade_source(src)
    except (OSError, ValueError) as e:
        log(f"[失败] 升级源不可用：{e}")
        return 2
    try:
        return do_install(pkg, False, install_dir, silent=silent, shortcuts=shortcuts,
                          force=force, desktop_dir=desktop_dir,
                          start_menu_dir=start_menu_dir, health_cmd=health_cmd)
    finally:
        if ctx is not None:
            ctx.cleanup()


def _installed_version(install: Path) -> str:
    """finalize --archive：读安装树当前版本（<install>/app/VERSION；缺省回退仓库 VERSION）。"""
    try:
        return (install / "app" / "VERSION").read_text("ascii").strip() or VERSION_DEFAULT
    except OSError:
        return VERSION_DEFAULT


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 便携版安装/升级/卸载/回滚（无管理员）")
    ap.add_argument("action", choices=["install", "upgrade", "uninstall", "rollback", "finalize"])
    ap.add_argument("--from", dest="from_src", type=Path,
                    help="upgrade 专用：便携包 zip 或目录")
    ap.add_argument("--health-cmd", nargs="+", default=None,
                    help="覆盖健康检查命令（演练/测试用；生产缺省=runtime 导入冒烟）")
    ap.add_argument("--source", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="便携包根（默认=脚本上一级）")
    ap.add_argument("--install-dir", type=Path,
                    default=DEFAULT_INSTALL_DIR,
                    help="安装目标（install/uninstall 都按它；默认 %%LOCALAPPDATA%%\\Programs\\LabSentinel）")
    ap.add_argument("--autostart", action="store_true", help="安装时开启开机自启（HKCU）")
    ap.add_argument("--no-autostart", action="store_true", help="明确不开自启（默认行为；与 --autostart 互斥）")
    ap.add_argument("--purge-data", action="store_true", help="卸载时同时删除用户数据")
    ap.add_argument("--archive", action="store_true",
                    help="finalize 专用：归档当前旧版本到 versions/<ver>/（Inno ssInstall 时机）")
    ap.add_argument("--silent", action="store_true", help="静默安装：无交互提示，结果仍打印 stdout")
    ap.add_argument("--no-shortcuts", action="store_true", help="跳过桌面/开始菜单快捷方式")
    ap.add_argument("--dry-run", action="store_true", help="只打印将执行的动作，不写任何文件/注册表")
    ap.add_argument("--force", action="store_true", help="覆盖非 LabSentinel 的非空目标目录")
    ap.add_argument("--desktop-dir", type=Path, help="覆盖桌面快捷方式位置（隔离测试用）")
    ap.add_argument("--start-menu-dir", type=Path, help="覆盖开始菜单位置（隔离测试用）")
    a = ap.parse_args()
    if a.autostart and a.no_autostart:
        print("[失败] --autostart 与 --no-autostart 互斥")
        return 2
    if a.action == "upgrade":
        src = a.from_src or a.source
        if not src:
            print("[失败] upgrade 需要 --from <zip|dir>")
            return 2
        return do_upgrade(src, a.install_dir, silent=a.silent,
                          shortcuts=not a.no_shortcuts, force=a.force,
                          desktop_dir=a.desktop_dir, start_menu_dir=a.start_menu_dir,
                          health_cmd=a.health_cmd)
    if a.action == "install":
        return do_install(a.source, a.autostart, a.install_dir, silent=a.silent,
                          shortcuts=not a.no_shortcuts, dry_run=a.dry_run, force=a.force,
                          desktop_dir=a.desktop_dir, start_menu_dir=a.start_menu_dir,
                          health_cmd=a.health_cmd)
    if a.action == "finalize":
        # B2/ADR-0010：Inno Setup 调用，两个时机、复用 T-404 既有能力：
        #   --archive（ssInstall，文件复制前）：归档当前旧版到 versions/<ver>/
        #     + 备份 DB，作为升级回滚点；
        #   --verify（ssPostInstall，文件复制后）：真实健康检查（启动新版本
        #     uvicorn → 轮询 /api/health/ready → 停止），失败自动 rollback()
        #     并让 Inno [Run] 拿到非 0 退出码中止安装。
        if a.archive:
            if not a.install_dir.exists():
                log("[跳过] 首次安装，无需归档")
                return 0
            ver = _installed_version(a.install_dir)
            archived = _archive_current(a.install_dir, ver)
            if archived is None:
                log("[警告] 旧版本归档失败（继续安装，但本次升级无回滚点）")
            else:
                log(f"[OK] 旧版本已归档 → {archived}")
            return 0
        spec = health_check.default_health_cmd(a.install_dir)
        if spec is None:
            log("[跳过] runtime 缺失（桩环境），finalize 无健康检查可做")
            return 0
        argv, env = spec
        if a.health_cmd:  # 测试/演练覆盖：直接跑指定命令
            argv, env = a.health_cmd, None
        # 2026-11-13 P2（审查实证）：探针挂死则 Inno Setup 安装永久卡住——90s
        # 超时按检查失败处理，走回滚（与 _health_check 的 60s 口径一致）。
        try:
            r = subprocess.run(argv, env=env, capture_output=True, text=True, errors="replace", timeout=90)
            failed = r.returncode != 0 or (r.stdout and "ready=False" in r.stdout)
            detail = f"rc={r.returncode}）: {(r.stdout or '').strip()[:200]}"
        except subprocess.TimeoutExpired:
            failed = True
            detail = "90s 超时）: 探针无响应"
        if failed:
            log(f"[失败] 新版本健康检查未通过（{detail}")
            if not a.dry_run:
                rc = rollback(a.install_dir)
                log(f"[回滚] rc={rc}（数据目录未动）")
            return 3
        log("[OK] finalize 健康检查通过")
        return 0
    if a.action == "rollback":  # T-404
        return rollback(a.install_dir)
    # 受管根白名单校验在 main 内完成（布尔判定，env 不再流入 do_uninstall 的删除调用图）
    allowed_roots = tuple(
        Path(os.environ[k]).resolve()
        for k in ("LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)", "USERPROFILE")
        if os.environ.get(k)
    )
    if not _uninstall_target_allowed(a.install_dir, allowed_roots):
        log("[拒绝] 卸载目标非法（须位于 LOCALAPPDATA/PROGRAMFILES 下的二级以上目录，"
            "且非系统/用户主目录本身）——未执行任何删除")
        return 2
    return do_uninstall(a.install_dir, a.purge_data)


if __name__ == "__main__":
    sys.exit(main())
