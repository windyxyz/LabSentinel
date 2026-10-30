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
import time
from datetime import datetime
from pathlib import Path

VERSION_DEFAULT = "2026.10"
RUN_KEY = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run"


def log(m=""):
    print(m, flush=True)


def ps(script: str) -> None:
    exe = shutil.which("powershell") or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    r = subprocess.run([exe, "-NoProfile", "-Command", script],
                       capture_output=True, text=True, encoding="mbcs", errors="replace")
    if r.returncode != 0:
        # PS 输出是 GBK：PYTHONUTF8=1 下不指定 mbcs 解码会崩（build_launcher_exe 同款修法）
        err = (r.stderr or "").strip()[:200]
        raise RuntimeError(f"PowerShell 失败: {err or f'exit {r.returncode}（多为路径不存在/权限）'}")


def desktop_dir() -> Path:
    exe = shutil.which("powershell") or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    r = subprocess.run([exe, "-NoProfile", "-Command",
                        '[Environment]::GetFolderPath("Desktop")'],
                       capture_output=True, text=True, encoding="mbcs", errors="replace")
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
    p = desktop / "LabSentinel.lnk"
    if p.exists():
        p.unlink()
    if smenu.exists():
        shutil.rmtree(smenu, ignore_errors=True)
    log("  快捷方式已删除")


def set_autostart(install: Path, on: bool) -> None:
    if on:
        val = f'"{install / "runtime" / "pythonw.exe"}" "{install / "scripts" / "portable_launcher.py"}"'
        r = subprocess.run(["reg", "add", RUN_KEY, "/v", "LabSentinel", "/t", "REG_SZ",
                            "/d", val, "/f"], capture_output=True, text=True)
        if r.returncode == 0:
            log("  开机自启：已开启（HKCU Run\\LabSentinel）")
        else:
            # 部分安全/启动项管理软件会锁定 Run 键（实测"拒绝访问"）——如实报告，不影响安装
            log(f"  开机自启：写入失败（{r.stderr.strip()[:60] or '拒绝访问'}）——"
                "可能被安全软件/启动项管理锁定；可手动把快捷方式放入 shell:startup")
    else:
        subprocess.run(["reg", "delete", RUN_KEY, "/v", "LabSentinel", "/f"],
                       capture_output=True)
        log("  开机自启：已关闭/清除")


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
               start_menu_dir: Path | None = None) -> int:
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
        log(f"[dry-run] 将执行（未写任何文件/注册表）：")
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
            subprocess.run([str(stopper), str(stop_script), "--stop"], capture_output=True)
        except OSError:
            pass  # runtime 损坏/非可执行（如测试假包）：升级按无服务继续
        time.sleep(1)  # 等句柄释放
    install.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, install, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "data",
                                                  "PENDING_DELETE"))
    ver_file = install / "app" / "VERSION"
    if not ver_file.exists():
        ver_file.write_text(VERSION_DEFAULT, encoding="ascii")
    log(f"  版本：{ver_file.read_text(encoding='ascii').strip()}"
        + ("（用户数据在数据目录，升级不受影响）" if upgrading else ""))
    if shortcuts:
        make_shortcuts(install, desktop=desktop_dir, start_menu=start_menu_dir)
    else:
        log("  快捷方式：已跳过（--no-shortcuts）")
    set_autostart(install, autostart)
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
    return 0


def _rmtree_retry(path: Path, attempts: int = 3) -> None:
    """删除带重试：刚被杀的进程句柄/Defender 实时扫描会短暂锁文件。"""
    for i in range(attempts):
        shutil.rmtree(path, ignore_errors=True)
        if not path.exists():
            return
        time.sleep(2)
    shutil.rmtree(path, ignore_errors=True)


def do_uninstall(install: Path, purge: bool) -> int:
    dbg = Path(os.environ["TEMP"]) / "ls_uninstall_trace.log"
    def trace(msg: str) -> None:
        # utf-8+replace：residue 可能含中文文件名，ascii 会让卸载器崩在调试日志上（实测）
        with open(dbg, "a", encoding="utf-8", errors="replace") as f:
            f.write(f"{datetime.now():%H:%M:%S}.{datetime.now().microsecond // 1000:03d} {msg}\n")
    trace(f"uninstall entry dir={install} purge={purge}")
    log(f"[卸载] {install}")
    exe = install / "LabSentinel.exe"
    if exe.exists():  # 任务55：优先 EXE 停止入口，回退 Python 启动器
        subprocess.run([str(exe), "--stop"], capture_output=True)
    else:
        subprocess.run([str(install / "runtime" / "python.exe"),
                        str(install / "scripts" / "portable_launcher.py"), "--stop"],
                       capture_output=True)
    trace("stopped")
    time.sleep(1)  # 给刚被 kill 的 uvicorn 句柄释放时间
    remove_shortcuts(install)  # 按 shortcuts.json 记录清理（自定义目录卸载才不残留）
    set_autostart(install, False)
    trace("shortcuts+autostart done")
    data = Path(os.environ["LOCALAPPDATA"]) / "LabSentinel"
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
        tmp_cmd = Path(os.environ["TEMP"]) / "ls_uninstall_cleanup.cmd"
        dbg = Path(os.environ["TEMP"]) / "ls_uninstall_cleanup.log"
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
        _rmtree_retry(data)
        log("  数据已删除（--purge-data）" + ("" if not data.exists() else "（部分文件仍被占用，重试后未清空）"))
    else:
        log(f"  数据保留在 {data}（含 config.env/事件/截图；确认不要可手动删除，或 --purge-data）")
    # 进程内只删未锁定部分（app/models/tools/scripts/LICENSE/NOTICE/入口 bat；
    # runtime 由下方哨兵守卫的延迟 rd 处理——不碰它）
    for name in ("app", "models", "tools", "scripts", "LICENSE", "NOTICE",
                 "LabSentinel.exe", "启动.bat", "停止.bat", "安装.bat", "卸载.bat"):
        p = install / name
        if not p.exists():
            continue
        if p.is_dir():
            _rmtree_retry(p)
        else:
            p.unlink(missing_ok=True)
    log("[卸载] 完成（runtime 残留将自动删除，最长约 60 秒）")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 便携版安装/卸载（无管理员）")
    ap.add_argument("action", choices=["install", "uninstall"])
    ap.add_argument("--source", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="便携包根（默认=脚本上一级）")
    ap.add_argument("--install-dir", type=Path,
                    default=Path(os.environ["LOCALAPPDATA"]) / "Programs" / "LabSentinel",
                    help="安装目标（install/uninstall 都按它；默认 %%LOCALAPPDATA%%\\Programs\\LabSentinel）")
    ap.add_argument("--autostart", action="store_true", help="安装时开启开机自启（HKCU）")
    ap.add_argument("--no-autostart", action="store_true", help="明确不开自启（默认行为；与 --autostart 互斥）")
    ap.add_argument("--purge-data", action="store_true", help="卸载时同时删除用户数据")
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
    if a.action == "install":
        return do_install(a.source, a.autostart, a.install_dir, silent=a.silent,
                          shortcuts=not a.no_shortcuts, dry_run=a.dry_run, force=a.force,
                          desktop_dir=a.desktop_dir, start_menu_dir=a.start_menu_dir)
    return do_uninstall(a.install_dir, a.purge_data)


if __name__ == "__main__":
    sys.exit(main())
