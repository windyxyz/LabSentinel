r"""安装器 UI 层（任务120）：安装/升级/卸载的用户交互（对话框+目录选择）。

分工：install_portable.py 是无交互引擎（do_install/do_uninstall，含测试）；本文件只套
用户对话流。驱动（LabSentinel.exe/portable_launcher.py）只管启停与首启形态选择。

用法（包内 runtime python；安装.bat / 控制面板 UninstallString 调用）:
    runtime\python.exe scripts\installer.py install --ui                  # 首装选目录/已装三选
    runtime\python.exe scripts\installer.py install --ui --source <包根>  # 指定源（默认包自身）
    runtime\python.exe scripts\installer.py uninstall --ui --install-dir <目录>
静默/自动化请直接用 install_portable.py（CI 与测试口径）。
"""
import argparse
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import install_portable  # noqa: E402

MB_OK, MB_OKCANCEL, MB_YESNO, MB_YESNOCANCEL = 0x0, 0x01, 0x04, 0x03
MB_ICONINFO, MB_ICONWARN = 0x40, 0x30
IDOK, IDCANCEL, IDYES, IDNO = 1, 2, 6, 7


def msg(text: str, title: str = "LabSentinel 安装器", style: int = MB_OK) -> int:
    import ctypes
    return ctypes.windll.user32.MessageBoxW(None, text, title, style)


def _ps(args: list[str]) -> str:
    exe = shutil.which("powershell") or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    r = subprocess.run([exe, *args], capture_output=True, text=True,
                       encoding="mbcs", errors="replace",
                       creationflags=install_portable.CREATE_NO_WINDOW)
    return (r.stdout or "").strip()


def pick_folder(default: Path) -> Path | None:
    """FolderBrowserDialog 选安装目录（PowerShell 承载，与 install_portable.ps 同款解码）。"""
    init = str(default).replace("'", "''")
    out = _ps(["-NoProfile", "-STA", "-Command",
               "Add-Type -AssemblyName System.Windows.Forms | Out-Null; "
               "$d = New-Object System.Windows.Forms.FolderBrowserDialog; "
               f"$d.SelectedPath = '{init}'; "
               "$d.Description = 'Select LabSentinel install folder'; "
               "if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath } else { '' }"])
    return Path(out) if out else None


def _version(install: Path) -> str:
    f = install / "app" / "VERSION"
    try:
        return f.read_text(encoding="ascii").strip()
    except (OSError, ValueError):
        return install_portable.VERSION_DEFAULT


def _start(install: Path) -> None:
    """从安装位置启动（exe 优先，缺则 pythonw 驱动）。"""
    exe = install / "LabSentinel.exe"
    try:
        if exe.exists():
            subprocess.Popen([str(exe)], cwd=str(install))
        else:
            subprocess.Popen([str(install / "runtime" / "pythonw.exe"),
                              str(install / "scripts" / "portable_launcher.py")], cwd=str(install))
    except OSError as e:
        msg(f"启动失败：{e}\n可稍后手动运行 {exe}", style=MB_OK | MB_ICONWARN)


def _wait_runtime_released(install: Path, max_s: int = 75) -> None:
    """清空重装前等延迟 rd 拿掉 PENDING_DELETE（哨兵守卫，最长 ~75s）。"""
    marker = install / "runtime" / "PENDING_DELETE"
    dl = time.time() + max_s
    while marker.exists() and time.time() < dl:
        time.sleep(2)


def install_ui(source: Path) -> int:
    loc = install_portable.installed_location()
    if loc is not None:
        rc = msg("检测到已安装的 LabSentinel：\n" + str(loc) + "\n\n"
                 "是(Y)：跳过安装，直接启动现有版本\n"
                 "否(N)：覆盖安装（程序更新，数据保留）\n"
                 "取消：清空数据重装（恢复出厂，需二次确认）",
                 style=MB_YESNOCANCEL | MB_ICONINFO)
        if rc == IDYES:
            _start(loc)
            return 0
        if rc == IDNO:
            r = install_portable.do_install(source, autostart=False, install_dir=loc, silent=True)
            if r != 0:
                msg("覆盖安装失败，详见控制台输出。", style=MB_OK | MB_ICONWARN)
            else:
                _start(loc)
            return r
        # IDCANCEL → 清空数据重装
        if msg("将删除全部数据并重装（事件/截图/配置不可恢复）。确定？",
               style=MB_YESNO | MB_ICONWARN) != IDYES:
            return 0
        install_portable.do_uninstall(loc, purge=True)
        _wait_runtime_released(loc)
        r = install_portable.do_install(source, autostart=False, install_dir=loc, silent=True)
        if r != 0:
            msg("重装失败，详见控制台输出。", style=MB_OK | MB_ICONWARN)
        else:
            _start(loc)
        return r

    # 未安装：选目录 → 安装 → 注册表（do_install 内含 set_uninstall_registry）→ 启动
    default = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "Programs" / "LabSentinel"
    picked = pick_folder(default)
    if picked is None:
        return 1
    target = picked if picked.name.lower() == "labsentinel" else picked / "LabSentinel"
    autostart = msg("安装后开机自动启动 LabSentinel？", style=MB_YESNO | MB_ICONINFO) == IDYES
    r = install_portable.do_install(source, autostart=autostart, install_dir=target, silent=True)
    if r != 0:
        msg("安装失败，详见控制台输出。", style=MB_OK | MB_ICONWARN)
        return r
    _start(target)
    return 0


def uninstall_ui(install: Path) -> int:
    if not install.exists():
        msg(f"未发现安装目录：\n{install}", style=MB_OK | MB_ICONWARN)
        return 0
    rc = msg("卸载 LabSentinel？\n\n"
             "是(Y)：卸载程序，保留数据（事件/截图/配置）\n"
             "否(N)：卸载程序并删除全部数据\n"
             "取消：不卸载", style=MB_YESNOCANCEL | MB_ICONINFO)
    if rc == IDCANCEL:
        return 0
    purge = False
    if rc == IDNO:
        purge = msg("确定删除全部数据（不可恢复）？", style=MB_YESNO | MB_ICONWARN) == IDYES
    return install_portable.do_uninstall(install, purge)


def main() -> int:
    ap = argparse.ArgumentParser(description="LabSentinel 安装器（交互 UI，任务120）")
    ap.add_argument("action", choices=["install", "uninstall"])
    ap.add_argument("--ui", action="store_true", help="交互模式（对话框）；缺省仅提示")
    ap.add_argument("--source", type=Path, default=Path(__file__).resolve().parent.parent,
                    help="便携包根（默认=脚本上一级）")
    ap.add_argument("--install-dir", type=Path,
                    default=Path(os.environ.get("LOCALAPPDATA", "")) / "Programs" / "LabSentinel")
    a = ap.parse_args()
    if not a.ui:
        print("交互模式请加 --ui；静默安装/卸载用 scripts/install_portable.py（CI 口径）")
        return 2
    if a.action == "install":
        return install_ui(a.source)
    return uninstall_ui(a.install_dir)


if __name__ == "__main__":
    sys.exit(main())
