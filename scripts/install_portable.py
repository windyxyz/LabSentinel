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
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"PowerShell 失败: {r.stderr.strip()[:200]}")


def desktop_dir() -> Path:
    exe = shutil.which("powershell") or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    r = subprocess.run([exe, "-NoProfile", "-Command",
                        '[Environment]::GetFolderPath("Desktop")'],
                       capture_output=True, text=True)
    return Path(r.stdout.strip() or (Path.home() / "Desktop"))


def make_shortcuts(install: Path) -> None:
    pythonw = install / "runtime" / "pythonw.exe"
    launcher = install / "scripts" / "portable_launcher.py"
    desktop = desktop_dir()
    smenu = Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "LabSentinel"
    smenu.mkdir(parents=True, exist_ok=True)

    def esc(s: str) -> str:
        """PowerShell 单引号字符串转义：' -> ''。"""
        return s.replace("'", "''")

    def lnk(path: Path, args: str) -> None:
        script = (
            f"$ws = New-Object -ComObject WScript.Shell; "
            f"$s = $ws.CreateShortcut('{esc(str(path))}'); "
            f"$s.TargetPath = '{esc(str(pythonw))}'; "
            f"$s.Arguments = '\"{esc(str(launcher))}\"'; "
            f"$s.WorkingDirectory = '{esc(str(install))}'; "
            f"$s.Save()")
        ps(script)

    lnk(desktop / "LabSentinel.lnk", f'"{launcher}"')
    lnk(smenu / "LabSentinel.lnk", f'"{launcher}"')
    lnk(smenu / "卸载.lnk",
        f'"{install / "scripts" / "install_portable.py"}" uninstall --purge-data')
    log(f"  快捷方式：桌面 + 开始菜单（{smenu.name}）")


def remove_shortcuts() -> None:
    desktop = desktop_dir()
    p = desktop / "LabSentinel.lnk"
    if p.exists():
        p.unlink()
    smenu = Path(os.environ["APPDATA"]) / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "LabSentinel"
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


def do_install(src: Path, autostart: bool) -> int:
    if not (src / "runtime" / "python.exe").exists():
        raise SystemExit(f"[失败] 源不是便携包（缺 runtime\\python.exe）：{src}")
    install = Path(os.environ["LOCALAPPDATA"]) / "Programs" / "LabSentinel"
    upgrading = install.exists()
    log(f"[安装] {'覆盖升级' if upgrading else '全新安装'}：{src} -> {install}")
    install.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, install, dirs_exist_ok=True,
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "data",
                                                  "PENDING_DELETE"))
    ver_file = install / "app" / "VERSION"
    if not ver_file.exists():
        ver_file.write_text(VERSION_DEFAULT, encoding="ascii")
    log(f"  版本：{ver_file.read_text(encoding='ascii').strip()}"
        + ("（用户数据在 %LOCALAPPDATA%\\LabSentinel，升级不受影响）" if upgrading else ""))
    make_shortcuts(install)
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
        "\"%HERE%runtime\\python.exe\" \"%HERE%scripts\\install_portable.py\" uninstall --purge-data %PURGE%\n"
        "pause\n",
        encoding="ascii")
    log(f"[安装] 完成。启动方式：桌面/开始菜单快捷方式，或 {install / '启动.bat'}")
    log("  卸载：开始菜单 卸载.lnk 或安装目录 卸载.bat（默认保留数据）")
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
    subprocess.run([str(install / "runtime" / "python.exe"),
                    str(install / "scripts" / "portable_launcher.py"), "--stop"],
                   capture_output=True)
    trace("stopped")
    time.sleep(1)  # 给刚被 kill 的 uvicorn 句柄释放时间
    remove_shortcuts()
    set_autostart(install, False)
    trace("shortcuts+autostart done")
    data = Path(os.environ["LOCALAPPDATA"]) / "LabSentinel"
    trace("hybrid done residue=" + ",".join(sorted(q.name for q in install.iterdir())))
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
                 "启动.bat", "停止.bat", "安装.bat", "卸载.bat"):
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
                    default=Path(os.environ["LOCALAPPDATA"]) / "Programs" / "LabSentinel")
    ap.add_argument("--autostart", action="store_true", help="安装时开启开机自启（HKCU）")
    ap.add_argument("--purge-data", action="store_true", help="卸载时同时删除用户数据")
    a = ap.parse_args()
    if a.action == "install":
        return do_install(a.source, a.autostart)
    return do_uninstall(a.install_dir, a.purge_data)


if __name__ == "__main__":
    sys.exit(main())
