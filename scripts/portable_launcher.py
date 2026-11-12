"""便携启动器（任务45）：双击 启动.bat → 无黑框拉起单端口服务 → 等就绪 → 开浏览器。

只用标准库，由 runtime/pythonw.exe 运行（无控制台）。职责：
  - 数据外置：%LOCALAPPDATA%\\LabSentinel\\{data,logs,config.env}（首次从 .env.template 生成并改绝对路径）
  - 端口：config.env 的 port（默认 8000），被占则 8100–8199 逐个试 bind，选定后写回
  - 单实例：logs/launcher.lock 记 "pid port"；存活且健康 → 只开浏览器不重复起服务
  - 启动：runtime\\python.exe -m uvicorn app.main:app（CREATE_NO_WINDOW，日志落 logs/server-<日期>.log）
  - --stop：按锁 PID 结束进程（先 taskkill 再 /F），删锁，退出码 0

环境变量 LS_LAUNCHER_NO_BROWSER=1 时不开浏览器（自动化测试用，也不弹失败 MessageBox）；
LS_NO_MESSAGEBOX=1 只关弹窗；LS_READY_TIMEOUT_S 覆盖就绪等待（默认 60s，任务119）。
"""
import json
import logging
import os
import socket
import subprocess
import sys

# 跨平台子进程文本编码：mbcs 仅 Windows 有（Linux CI 导入即 LookupError）——
# 非 Windows 用 utf-8 + errors=replace 兜底；本模块功能仍以 Windows 为主。
PROC_ENCODING = "mbcs" if sys.platform == "win32" else "utf-8"
import time
import urllib.request
import webbrowser
from datetime import datetime
from pathlib import Path

APP_ROOT = Path(__file__).resolve().parent.parent      # scripts/ 的上一级 = 便携包根
RUNTIME = APP_ROOT / "runtime"
SERVER = APP_ROOT / "app" / "server"
MEDIA = APP_ROOT / "media"
TEMPLATE = SERVER / ".env.template"
DATA_ROOT = Path(os.environ.get("LOCALAPPDATA", str(APP_ROOT / "_userdata"))) / "LabSentinel"
DATA = DATA_ROOT / "data"
LOGS = DATA_ROOT / "logs"
CONFIG = DATA_ROOT / "config.env"
LOCK = LOGS / "launcher.lock"
_SERVER_PROCS: list = []  # uvicorn Popen 句柄（就绪循环 poll() 用）
# 2026-11-11 冒烟实证修复：go2rtc 锁曾写入 launcher.lock（JSON 覆盖 "pid port"
# 文本），导致单实例锁被毁——下次启动 read_lock() 抛 ValueError 当陈旧锁、
# smoke_portable 步骤 3a 90s 等不到合法锁、go2rtc 也无法正确回收。拆分到独立
# 文件，launcher.lock 永远只存 "pid port"（T-101/T-202 契约）。
GO2RTC_LOCK = LOGS / "go2rtc.lock"


def stop_go2rtc_if_any(legacy: dict | None = None) -> None:
    """按 go2rtc.lock 停止托管的 go2rtc（强杀+消亡窗核实）。

    do_stop 主路径 / 无锁早退 / do_start 超时清理三处共用——2026-11-13 P2（审查
    实证）：失败路径此前只停 uvicorn 不停 go2rtc → 孤儿常驻 1984，且后续 --stop
    在无 launcher.lock 时早退永远够不到它。legacy：旧版把 JSON 写在 launcher.lock
    里的残留数据（do_stop 删锁前读出传入，兼容升级路径）。
    """
    lock_data: dict = {}
    if GO2RTC_LOCK.exists():
        try:
            lock_data = json.loads(GO2RTC_LOCK.read_text(encoding="ascii"))
        except (OSError, ValueError):
            lock_data = {}
    elif isinstance(legacy, dict):
        lock_data = legacy
    g2r_pid = (lock_data.get("go2rtc") or {}).get("pid") if isinstance(lock_data, dict) else None
    GO2RTC_LOCK.unlink(missing_ok=True)
    if g2r_pid and pid_alive(g2r_pid):
        subprocess.run(["taskkill", "/PID", str(g2r_pid), "/F"], capture_output=True,
                       creationflags=CREATE_NO_WINDOW)
        deadline = time.time() + STOP_DEATH_WAIT_S / 2
        while pid_alive(g2r_pid) and time.time() < deadline:
            time.sleep(0.25)
        if pid_alive(g2r_pid):
            log.warning("go2rtc pid=%d 强杀后仍存活，请人工处理", g2r_pid)
        else:
            log.info("已停止 go2rtc pid=%d", g2r_pid)
PORT_DEFAULT = 8000
# 非 Windows creationflags 非法（Linux CI 导入本模块的测试会炸）——置 0
CREATE_NO_WINDOW = 0x08000000 if sys.platform == "win32" else 0

# 播种单源（任务51）：schema/演示摄像头/区域/规则统一在 scripts/seed_demo.py 维护
sys.path.insert(0, str(Path(__file__).resolve().parent))
import install_portable  # noqa: E402  任务120：安装位置检测/注册表卸载项共用
import seed_demo  # noqa: E402

log = logging.getLogger("launcher")


def setup_logging() -> None:
    LOGS.mkdir(parents=True, exist_ok=True)
    logging.basicConfig(
        filename=str(LOGS / f"launcher-{datetime.now():%Y%m%d}.log"),
        level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")


def read_config() -> dict:
    cfg = {}
    if CONFIG.exists():
        for ln in CONFIG.read_text(encoding="utf-8").splitlines():
            ln = ln.strip()
            if ln and not ln.startswith("#") and "=" in ln:
                k, v = ln.split("=", 1)
                cfg[k.strip()] = v.strip()
    return cfg


def write_config_port(port: int) -> None:
    lines = CONFIG.read_text(encoding="utf-8").splitlines() if CONFIG.exists() else []
    out, done = [], False
    for ln in lines:
        if ln.strip().startswith("port="):
            out.append(f"port={port}"); done = True
        else:
            out.append(ln)
    if not done:
        out.append(f"port={port}")
    CONFIG.write_text("\n".join(out) + "\n", encoding="utf-8")


def seed_if_empty() -> None:
    """首次运行播种（任务47/51）：逻辑单源 scripts/seed_demo.py——cameras 表为空且 media 在
    → 播 4 路演示摄像头/区域/规则。用户已有数据绝不覆盖；media 缺失只警告不阻塞。"""
    inserted = seed_demo.seed_database(DATA / "labsentinel.db", MEDIA)
    if inserted:
        log.info("首次播种完成：%d 路演示摄像头 + 区域/规则（media=%s）", inserted, MEDIA)
    elif not all((MEDIA / fn).exists() for _, fn, _ in seed_demo.SEED_CAMERAS):
        log.warning("media/ 演示视频缺失，跳过播种（不影响启动）")
    else:
        log.info("已有摄像头数据，跳过播种")


def repair_media_paths() -> None:
    """便携包被移动/改名后，播种摄像头可能指向旧 media 路径；按文件名在 media/ 重新定位。"""
    import sqlite3
    db_path = DATA / "labsentinel.db"
    if not db_path.exists():
        return
    db = sqlite3.connect(db_path)
    try:
        fixed = 0
        for cid, src in db.execute("select id, source from cameras where type='file'").fetchall():
            name = Path(src).name if src else ""
            target = MEDIA / name
            if name and target.exists() and (not src or not Path(src).exists()):
                db.execute("update cameras set source=? where id=?", (target.as_posix(), cid))
                fixed += 1
        if fixed:
            db.commit()
            log.info("修复演示摄像头媒体路径 %d 条（便携包移动后自愈）", fixed)
    finally:
        db.close()


def first_run(cfg: dict) -> None:
    """首次运行：建数据目录；从模板生成 config.env，模板相对路径改便携包绝对路径。"""
    for d in (DATA, DATA / "snapshots", DATA / "videos", DATA / "annotation_pool", LOGS):
        d.mkdir(parents=True, exist_ok=True)
    seed_if_empty()
    repair_media_paths()
    # T-104（Q-02）：升级路径也要补生成令牌（旧 config.env 无 LS_ADMIN_TOKEN）
    try:
        sys.path.insert(0, str(SERVER))
        from app.auth_setup import ensure_admin_token
        ensure_admin_token(CONFIG, DATA)
    except Exception as e:  # noqa: BLE001 令牌生成失败不阻断启动（退回无令牌模式 + 日志）
        log.warning("首启/升级令牌生成失败（退回无令牌模式）: %s", e)
    if CONFIG.exists():
        return
    text = TEMPLATE.read_text(encoding="utf-8") if TEMPLATE.exists() else ""
    model_abs = (APP_ROOT / "models" / "yolo11n_ppe.onnx").as_posix()
    text = "\n".join(ln for ln in text.splitlines() if not ln.strip().startswith("LS_PPE_MODEL="))
    text += f"\nLS_PPE_MODEL={model_abs}\nport={cfg.get('port', PORT_DEFAULT)}\n"
    CONFIG.write_text(text, encoding="utf-8")
    log.info("首次运行：config.env 已生成 %s", CONFIG)
    # T-104（Q-02）：首启/升级自动生成管理令牌（只动 LS_ADMIN_TOKEN 一行，其余逐字保留）
    try:
        sys.path.insert(0, str(SERVER))
        from app.auth_setup import ensure_admin_token
        ensure_admin_token(CONFIG, DATA)
    except Exception as e:  # noqa: BLE001 令牌生成失败不阻断启动（退回无令牌模式 + 日志）
        log.warning("首启令牌生成失败（退回无令牌模式）: %s", e)


def pick_port(cfg: dict) -> int:
    want = int(cfg.get("port", PORT_DEFAULT))

    def free(p):
        # 2026-10-30 审阅修复：Windows 上 SO_REUSEADDR 允许绑到已被监听的端口，探测恒"空闲"，
        # 端口漂移机制被完全架空——必须用 SO_EXCLUSIVEADDRUSE（独占语义）才能测出真占用
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            try:
                s.bind(("127.0.0.1", p))
                return True
            except OSError:
                return False

    if free(want):
        return want
    for p in range(8100, 8200):
        if free(p):
            log.info("端口 %d 被占用，自动改用 %d", want, p)
            return p
    raise SystemExit("8100-8199 全部被占用")


def health(port: int, timeout: float = 2.0) -> bool:
    """就绪探针（B-28）：用 /api/health/live（不触发模型加载）。
    /api/system 现在也不加载模型，但 live 更轻、语义就是"进程活着"。"""
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/health/live", timeout=timeout) as r:
            return r.status == 200
    except OSError:
        return False


def read_lock() -> tuple[int, int] | None:
    try:
        text = LOCK.read_text(encoding="ascii").strip()
        # 兼容 2026-11-11 之前被 go2rtc JSON 覆盖的旧锁：尝试从 JSON 里取不到
        # "pid port" 就当陈旧锁（不再把 JSON 结构误判为有效单实例锁）。
        if text.startswith("{"):
            return None
        pid, port = text.split()
        return int(pid), int(port)
    except (OSError, ValueError):
        return None


def pid_alive(pid: int) -> bool:
    """OpenProcess 判活（不用 tasklist 解析：其输出是 ANSI/GBK，在 UTF-8 模式 Python 下
    读取线程直接崩、stdout 变空，会把活进程误判为死——02:13 实测翻车一次）。"""
    import ctypes
    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    h = ctypes.windll.kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not h:
        return False
    ctypes.windll.kernel32.CloseHandle(h)
    return True


# 任务110：锁内 PID 必须真的是 LabSentinel 的 python 进程才允许 kill。
# 锁里记的是服务进程（uvicorn），不是启动器自身——三个标记都要认。
# 2026-11-12：服务改由探针 --serve 启动（cmdline=probe_runtime.py --serve …），
# 旧两标记匹配不到导致 --stop 误判"非 LabSentinel 进程"拒绝停止（冒烟步骤 6 实证：
# rc=0 但服务进程存活、端口持续 LISTEN）。
LAUNCHER_PROC_NAMES = ("python.exe", "pythonw.exe")
LAUNCHER_PROC_MARKERS = ("portable_launcher.py", "uvicorn app.main:app",
                         "probe_runtime.py")
# --stop 强杀后的进程消亡窗（秒）：taskkill /F 是异步终止，杀后立即查活会把
# 垂死进程误判存活。测试可 monkeypatch 此值缩短等待。
STOP_DEATH_WAIT_S = 5.0
_PS_EXE: str | None = None  # powershell.exe 绝对路径缓存（见 _powershell_exe）


def _powershell_exe() -> str:
    """powershell.exe 绝对路径——冒烟/便携环境 PATH 可能只有 System32
    （smoke_portable 剥离 PATH），裸 "powershell" 找不到 → 身份核实恒失败 →
    --stop 永远拒绝停止（2026-11-12 冒烟步骤 6 实证根因）。"""
    global _PS_EXE
    if _PS_EXE is None:
        cand = (Path(os.environ.get("SystemRoot", r"C:\Windows"))
                / "System32" / "WindowsPowerShell" / "v1.0" / "powershell.exe")
        _PS_EXE = str(cand) if cand.exists() else "powershell"
    return _PS_EXE


def _query_process(pid: int) -> str:
    """PowerShell 查进程"Name|CommandLine"（mbcs=ANSI，PS 重定向输出默认代码页）。

    返回空串 = 查询失败（权限/PS 不可用/超时/进程已消失），调用方按"身份不明"处理。
    """
    try:
        r = subprocess.run(
            [_powershell_exe(), "-NoProfile", "-NonInteractive", "-Command",
             f'$p = Get-CimInstance Win32_Process -Filter "ProcessId={int(pid)}" | '
             'Select-Object Name,CommandLine; if ($p) { "$($p.Name)|$($p.CommandLine)" }'],
            capture_output=True, text=True, encoding=PROC_ENCODING, errors="replace",
            timeout=5, creationflags=CREATE_NO_WINDOW)
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return (r.stdout or "").strip() if r.returncode == 0 else ""


def _pid_matches_launcher(pid: int) -> bool | None:
    """PID 复用防护（任务110）：锁残留后 PID 可能已被系统复用给无关进程。

    True=确认是 LabSentinel 的 python/pythonw；False=确认不是；
    None=身份核实失败（宁可拒绝杀也不能误杀，锁路径交给用户手动处理）。
    """
    out = _query_process(pid)
    if not out or "|" not in out:
        return None
    name, _, cmdline = out.partition("|")
    if name.strip().lower() not in LAUNCHER_PROC_NAMES:
        return False
    return any(marker in cmdline for marker in LAUNCHER_PROC_MARKERS)


def start_go2rtc(cfg: dict) -> int | None:
    """T-202（B-05）：托管 go2rtc.exe（生命周期同服务）。

    存在且 127.0.0.1:1984 未被占才拉起；yaml 由服务端/deploy 生成（回环+鉴权）。
    返回 pid（写入锁文件），None=无需托管（缺 exe 或端口已占）。"""
    exe = APP_ROOT / "tools" / "go2rtc.exe"
    if not exe.exists():
        log.info("tools/go2rtc.exe 不存在——实时画面不可用（其余功能不受影响）")
        return None
    import socket as _s
    probe = _s.socket()
    try:
        probe.bind(("127.0.0.1", 1984))
    except OSError:
        log.info("1984 已被占用——假定 go2rtc 已在运行，不重复拉起")
        return None
    finally:
        probe.close()
    logf = LOGS / f"go2rtc-{datetime.now():%Y%m%d}.log"
    # 2026-11-11：无控制台父进程（C# winexe → pythonw）下继承的句柄无效——
    # stdin/stdout/stderr 全部 DEVNULL（go2rtc 自身写 <包>/tools/go2rtc.log）。
    proc = subprocess.Popen(
        [str(exe), "-config", str(APP_ROOT / "tools" / "go2rtc.yaml")],
        cwd=str(APP_ROOT), stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    log.info("go2rtc pid=%d 日志=%s", proc.pid, logf)
    return proc.pid


def write_go2rtc_lock(pid: int) -> None:
    """go2rtc 锁独立文件（绝不覆盖 launcher.lock 的 "pid port" 契约）。"""
    try:
        GO2RTC_LOCK.write_text(json.dumps({"go2rtc": {"pid": pid}}), encoding="ascii")
    except OSError as e:
        log.warning("go2rtc 锁写入失败: %s", e)


def start_server(port: int, cfg: dict) -> int:
    data = str(DATA)
    env = {**os.environ,
           "LS_DATA_DIR": data,
           "LS_DB_PATH": os.path.join(data, "labsentinel.db"),
           "LS_SNAPSHOT_DIR": os.path.join(data, "snapshots"),
           "LS_VIDEO_DIR": os.path.join(data, "videos"),
           "LS_ANNOTATION_POOL_DIR": os.path.join(data, "annotation_pool"),
           "LS_PPE_MODEL": (APP_ROOT / "models" / "yolo11n_ppe.onnx").as_posix(),
           "LS_MODEL_FP32": (APP_ROOT / "models" / "yolo11n.onnx").as_posix(),
           "LS_WEB_DIST": (APP_ROOT / "app" / "web").as_posix(),
           # 任务119：便携布局下 config.py 的 BASE_DIR 推导会落到 <包>/app/tools/go2rtc.yaml
           # （源码模式才是 <根>/tools），显式指回包根 tools/，避免 go2rtc 冷启动读错位置
           "LS_GO2RTC_YAML": (APP_ROOT / "tools" / "go2rtc.yaml").as_posix(),
           "PYTHONUTF8": "1"}
    # R2-304: bundled experimental rust analyzer (bin/ls-analyzer.exe) — the
    # sidecar resolves it itself via analyzer_sidecar._default_exe(), but an
    # explicit env var keeps the contract obvious and overridable by config.env.
    _sidecar = APP_ROOT / "bin" / "ls-analyzer.exe"
    if _sidecar.is_file():
        env.setdefault("LS_ANALYZER_EXE", str(_sidecar))
        _ort = APP_ROOT / "bin" / "onnxruntime.dll"
        if _ort.is_file():
            env.setdefault("ORT_DYLIB_PATH", str(_ort))
    for k, v in cfg.items():          # config.env 里的 LS_* 透传（用户可加告警渠道等）
        if k.startswith("LS_"):
            env[k] = v
    # R2-102/N-03: --host comes from the same config.env LS_HOST that pydantic
    # settings reads. No split between "what uvicorn binds" and "what validation
    # assumes". Default stays loopback; LAN users must set a token (FATAL gate).
    bind_host = cfg.get("LS_HOST", "127.0.0.1").strip() or "127.0.0.1"
    server_log = LOGS / f"server-{datetime.now():%Y%m%d}.log"
    # 2026-11-11 冒烟实证修复：日志句柄必须在子进程生命周期内保持打开。
    # 旧实现 `with open(...) as lf: Popen(stdout=lf)` 在 Popen 返回后立即关闭 lf，
    # pythonw（无控制台）父进程下子进程的 stdout 句柄随之失效——uvicorn 启动即
    # 静默退出（零日志、端口不监听，冒烟 60s 等不到就绪）。句柄改为模块级持有，
    # 随启动器进程存活；日志按天追加，多日志句柄最多每天一个，可接受。
    with open(server_log, "ab") as lf:
        lf.write(f"\n===== launcher 启动 {datetime.now():%Y-%m-%d %H:%M:%S} host={bind_host} port={port} =====\n".encode())
    # 2026-11-11 冒烟实证：C# winexe → pythonw（无控制台）链下，Python 文件
    # 对象句柄继承给 embed Python 子进程时无效，uvicorn 启动即 returncode=1
    # 且零日志。重定向改由 cmd.exe 在 OS 层完成（句柄由 cmd 创建，子进程继承
    # 有效），启动异常也能落进 server-<日期>.log 供排障。
    # 2026-11-12 冒烟排障：无控制台进程链（C# winexe → pythonw）下子进程
    # stdout/stderr 继承句柄不可靠（崩溃零输出、returncode=1 黑箱）。改由随包
    # 探针 probe_runtime.py --serve 启动 uvicorn——任何启动异常写
    # <LS_DATA_DIR>/logs/serve-boot-error.txt 后非零退出；就绪循环 poll() 可据此
    # 拿到确凿原因。绑定主机经 LS_HOST 传递（探针内读同一变量，单一来源）。
    env["LS_HOST"] = bind_host
    env["LS_PORT"] = str(port)
    proc = subprocess.Popen(
        [str(RUNTIME / "python.exe"), str(APP_ROOT / "scripts" / "probe_runtime.py"),
         "--app-dir", str(SERVER), "--serve", str(port)],
        cwd=str(SERVER), env=env, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    _SERVER_PROCS.append(proc)  # 保留 Popen：就绪循环用 poll() 判活（比 OpenProcess 可靠）
    log.info("uvicorn pid=%d host=%s port=%d 日志=%s", proc.pid, bind_host, port, server_log)
    return proc.pid


def open_browser(port: int) -> None:
    if os.environ.get("LS_LAUNCHER_NO_BROWSER"):
        log.info("LS_LAUNCHER_NO_BROWSER=1，跳过打开浏览器")
        return
    webbrowser.open(f"http://127.0.0.1:{port}/")


def ready_timeout_s() -> int:
    """就绪等待秒数（任务119）：默认 60（冷启动实测 ~29s，30s 太紧会误判失败）。"""
    raw = os.environ.get("LS_READY_TIMEOUT_S", "60")
    try:
        return max(10, int(raw))
    except ValueError:
        return 60


# ---- 任务120：安装/便携双形态驱动逻辑 ----
MODE_FILE = DATA_ROOT / "launch.mode"  # 内容 "portable"=用户明确选便携；安装形态以注册表为准


def decide_launch_mode(installed: Path | None, mode_file: Path, unattended: bool,
                       app_root: Path | None = None) -> str:
    """启动决策（纯函数，可测）→ 'installed' | 'portable' | 'ask'。

    - 无人值守（自动化/冒烟）：绝不弹窗，注册表/标记只用于"转启安装副本"这一无交互路径，
      否则 59 冒烟的隔离语义会被真机残留安装破坏——无人值守一律便携。
    - 交互 + 已安装 → 安装副本（用户装完再点包，不应把包当便携跑——120 核心诉求）；
      **例外：自己就是安装副本时不得转启"自己"**（否则安装版双击 → 自我接力 fork 炸弹，
      2026-10-31 VM 实测：每个世代从 pythonw 查注册表都闪一个 reg.exe 黑窗，无限循环）；
    - 交互 + 无安装 + 无标记 → 首启三选；标记 portable → 便携。
    """
    if unattended:
        return "portable"
    if installed is not None:
        root = Path(app_root) if app_root is not None else APP_ROOT
        try:
            self_installed = root.exists() and installed.exists() and root.samefile(installed)
        except OSError:
            self_installed = False
        return "portable" if self_installed else "installed"
    if mode_file.exists() and mode_file.read_text(encoding="ascii").strip() == "portable":
        return "portable"
    return "ask"


def ask_first_run() -> str:
    """首启三选（MessageBox，标准库）→ 'portable' | 'install' | 'cancel'。"""
    try:
        import ctypes
        rc = ctypes.windll.user32.MessageBoxW(
            None,
            "LabSentinel 首次启动\n\n"
            "是(Y)：仅便携运行（不安装，随时可删包走人）\n"
            "否(N)：安装到本机（开始菜单/控制面板卸载，双击包内程序自动转到安装版）\n"
            "取消：退出",
            "LabSentinel", 0x03 | 0x40)  # MB_YESNOCANCEL | MB_ICONINFORMATION
        return {6: "portable", 7: "install"}.get(rc, "cancel")  # IDYES/IDNO/其余=cancel
    except Exception:  # noqa: BLE001  session0/无桌面：默认便携（与旧行为一致）
        log.info("首启选择弹窗失败（无桌面会话?），按便携继续")
        return "portable"


def relaunch_installed(install: Path) -> int:
    """转启安装副本（驱动自身退出）。失败回落便携并留痕。"""
    exe = install / "LabSentinel.exe"
    try:
        subprocess.Popen([str(exe)], cwd=str(install), creationflags=CREATE_NO_WINDOW)
        log.info("已转启安装副本：%s", install)
        return 0
    except OSError as e:
        log.warning("转启安装副本失败（%s），继续便携启动", e)
        return -1


def stop_process(pid: int) -> None:
    """停止服务进程（先雅停后强杀，与 --stop 一致；核实真死并落日志）。"""
    subprocess.run(["taskkill", "/PID", str(pid)], capture_output=True,
                   creationflags=CREATE_NO_WINDOW)
    time.sleep(2)
    subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True,
                   creationflags=CREATE_NO_WINDOW)
    deadline = time.time() + STOP_DEATH_WAIT_S
    while pid_alive(pid) and time.time() < deadline:
        time.sleep(0.25)
    if pid_alive(pid):
        log.warning("强杀后 pid=%d 仍存活（端口可能仍被占用），请人工处理", pid)


def do_start() -> int:
    setup_logging()
    log.info("===== launcher start =====")
    if not (RUNTIME / "python.exe").exists():
        log.error("找不到 runtime\\python.exe：%s", RUNTIME)
        return 1

    # 任务120：安装/便携双形态决策（无人值守不弹窗，语义见 decide_launch_mode）
    unattended = bool(os.environ.get("LS_LAUNCHER_NO_BROWSER") or os.environ.get("LS_NO_MESSAGEBOX"))
    installed = install_portable.installed_location()
    mode = decide_launch_mode(installed, MODE_FILE, unattended, app_root=APP_ROOT)
    log.info("启动形态：%s", mode)
    if mode == "ask":
        choice = ask_first_run()
        if choice == "cancel":
            return 0
        if choice == "portable":
            try:
                MODE_FILE.parent.mkdir(parents=True, exist_ok=True)
                MODE_FILE.write_text("portable", encoding="ascii")  # 选便携才落标记
            except OSError as e:
                log.warning("模式标记写入失败: %s", e)
        else:  # install：安装器接管全程（含装完启动），驱动让位退出——不再转启，避免双实例
            installer = APP_ROOT / "scripts" / "installer.py"
            log.info("拉起安装器（交互）...")
            return subprocess.call([str(RUNTIME / "python.exe"), str(installer), "install", "--ui"])
    elif mode == "installed":
        if relaunch_installed(installed) == 0:
            return 0
        log.warning("已安装但转启失败，按便携继续")

    cfg = read_config()
    first_run(cfg)
    cfg = read_config()

    lock = read_lock()
    if lock and pid_alive(lock[0]):
        # 服务已在运行：健康检查重试几次（首次启动模型加载可能 >2s，单次失败≠僵死——
        # 2026-10-30 审阅：单次失败即重启会造成双实例共写同一库）
        for _ in range(3):
            if health(lock[1]):
                log.info("服务已在运行 pid=%d port=%d，直接打开浏览器", *lock)
                open_browser(lock[1])
                return 0
            time.sleep(2)
        log.info("锁内 pid=%d 存活但健康检查连续失败——先停止旧实例（防双实例共写库）"
                 "再清锁重启", lock[0])
        try:
            stop_process(lock[0])
        except Exception as e:  # noqa: BLE001
            log.warning("停止旧实例失败: %s", e)
        LOCK.unlink(missing_ok=True)
    elif lock:
        LOCK.unlink(missing_ok=True)

    port = pick_port(cfg)
    write_config_port(port)
    g2r = start_go2rtc(cfg)  # T-202（B-05）：便携版实时画面——启动器托管 go2rtc
    pid = start_server(port, cfg)
    try:
        LOCK.write_text(f"{pid} {port}", encoding="ascii")
    except OSError as e:
        log.warning("锁写入失败（重试一次）: %s", e)
        time.sleep(0.5)
        LOCK.write_text(f"{pid} {port}", encoding="ascii")

    if g2r:
        write_go2rtc_lock(g2r)
    t0 = time.monotonic()
    timeout_s = ready_timeout_s()
    next_log = 5.0
    while time.monotonic() - t0 < timeout_s:
        if health(port):
            log.info("服务就绪（%.1fs）http://127.0.0.1:%d/", time.monotonic() - t0, port)
            open_browser(port)
            return 0
        waited = time.monotonic() - t0
        # 2026-11-11 冒烟实证：子进程可能启动即崩（零日志、端口不监听），
        # 旧循环只打"等待中"分不清"慢"与"死"——每次进展日志附带 poll 状态，
        # 一旦发现进程已退出立即失败，不再空等 60s（排障信息也可据此定位）。
        if waited >= next_log:
            proc = _SERVER_PROCS[-1] if _SERVER_PROCS else None
            rc = proc.poll() if proc is not None else None
            log.info("等待服务就绪中 %.0fs/%ds（服务进程 pid=%d poll=%s）",
                     waited, timeout_s, pid, rc)
            if rc is not None:
                log.error("服务进程已退出（returncode=%s），请查看 %s",
                          rc, LOGS / f"server-{datetime.now():%Y%m%d}.log")
                break
            next_log += 5.0
        time.sleep(1)
    log.error("%ds 内未就绪，请查看 %s", timeout_s, LOGS / f"server-{datetime.now():%Y%m%d}.log")
    # 任务119：无人值守（自动化/冒烟）不弹 MessageBox，只落日志；仍要清理子进程与锁，
    # 避免 30s/60s 判负后残留 uvicorn 占着端口和库
    if os.environ.get("LS_LAUNCHER_NO_BROWSER") or os.environ.get("LS_NO_MESSAGEBOX"):
        log.error("无人值守模式（NO_BROWSER/NO_MESSAGEBOX），跳过弹窗，返回非 0")
    else:
        try:
            import ctypes
            ctypes.windll.user32.MessageBoxW(
                None, f"服务 {timeout_s} 秒内未就绪。\n日志：{LOGS}", "LabSentinel 启动失败", 0x10)
        except Exception:  # noqa: BLE001
            pass
    log.info("超时清理：停止服务进程 pid=%d 并删除锁", pid)
    try:
        stop_process(pid)
    except Exception as e:  # noqa: BLE001
        log.warning("停止服务进程失败: %s", e)
    LOCK.unlink(missing_ok=True)
    stop_go2rtc_if_any()  # 2026-11-13 P2：超时清理此前漏掉 go2rtc（孤儿占 1984）
    return 1


def do_stop() -> int:
    setup_logging()
    lock = read_lock()
    if not lock:
        stop_go2rtc_if_any()  # 2026-11-13 P2：无 launcher.lock 时也要回收 go2rtc
        print("未发现运行中的 LabSentinel（无锁文件）。")
        return 0
    pid, port = lock
    if not pid_alive(pid):
        LOCK.unlink(missing_ok=True)
        print("锁已陈旧（进程不存在），已清理锁文件。")
        return 0
    verdict = _pid_matches_launcher(pid)
    if verdict is None:
        # 身份核实失败（权限/PS 不可用/超时）：锁保留，让下次重试仍能走正常停止路径
        print(f"无法核实锁内 pid={pid} 的进程身份（PowerShell 查询失败），为防误杀未执行停止。")
        print(f"锁文件：{LOCK}（确认服务已停后可手动删除）")
        log.warning("进程身份查询失败，拒绝停止 pid=%d，锁保留", pid)
        return 0
    if not verdict:
        # 确认不是我们的进程：锁已陈旧、PID 已被复用——删锁，绝不杀无关进程（任务110）
        print(f"锁内 pid={pid} 不是 LabSentinel 的 python 进程（锁已陈旧，PID 已被复用），未杀进程。")
        print("已清理锁文件；若服务确实还在运行，请用任务管理器定位后再处理。")
        LOCK.unlink(missing_ok=True)
        log.warning("拒绝停止 pid=%d（进程名/命令行不匹配），已删除陈旧锁", pid)
        return 0
    subprocess.run(["taskkill", "/PID", str(pid)], capture_output=True,
                   creationflags=CREATE_NO_WINDOW)
    time.sleep(2)
    subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True,
                   creationflags=CREATE_NO_WINDOW)
    # 2026-11-13 实证（CI dml 冒烟步骤 6 残留 PID）：taskkill 退出码被吞会让
    # 「已停止」变成谎言——强杀是异步终止，必须核实进程真死（消亡窗+重试一次）
    # 才删锁；仍存活则锁保留（下次 --stop 可重试）并如实返回非 0。
    for _ in range(2):
        if not pid_alive(pid):
            break
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True,
                       creationflags=CREATE_NO_WINDOW)
        deadline = time.time() + STOP_DEATH_WAIT_S / 2
        while pid_alive(pid) and time.time() < deadline:
            time.sleep(0.25)
    if pid_alive(pid):
        log.error("强杀后 pid=%d 仍存活，锁保留（服务可能仍占端口 %d）", pid, port)
        print(f"[失败] 服务进程 pid={pid} 强杀后仍存活；锁已保留，"
              "请重试 --stop 或用任务管理器处理后手动删除锁。")
        return 1
    LOCK.unlink(missing_ok=True)
    log.info("已停止 pid=%d port=%d", pid, port)
    print(f"已停止 LabSentinel（pid={pid}, port={port}）。")

    # T-202：go2rtc 与服务同生命周期——旧版把 JSON 写在 launcher.lock 里，必须
    # 先读后删（原实现先 unlink 再回读，legacy 路径永远读不到）；统一走助手。
    try:
        legacy = json.loads(LOCK.read_text(encoding="ascii")) if LOCK.exists() else {}
    except (OSError, ValueError):
        legacy = {}
    stop_go2rtc_if_any(legacy=legacy if isinstance(legacy, dict) else None)
    return 0


if __name__ == "__main__":
    try:
        # 停止信号：argv --stop（脚本直调）或 LS_PORTABLE_STOP=1（C# 启动器经环境变量传入）
        stop = "--stop" in sys.argv or os.environ.get("LS_PORTABLE_STOP") == "1"
        sys.exit(do_stop() if stop else do_start())
    except SystemExit:
        raise
    except Exception:  # noqa: BLE001  pythonw 无控制台：异常必须落日志否则无声死亡
        import traceback
        try:
            setup_logging()
            log.error("launcher 异常:\n%s", traceback.format_exc())
        except Exception:  # noqa: BLE001
            pass
        sys.exit(1)
