"""便携启动器（任务45）：双击 启动.bat → 无黑框拉起单端口服务 → 等就绪 → 开浏览器。

只用标准库，由 runtime/pythonw.exe 运行（无控制台）。职责：
  - 数据外置：%LOCALAPPDATA%\\LabSentinel\\{data,logs,config.env}（首次从 .env.template 生成并改绝对路径）
  - 端口：config.env 的 port（默认 8000），被占则 8100–8199 逐个试 bind，选定后写回
  - 单实例：logs/launcher.lock 记 "pid port"；存活且健康 → 只开浏览器不重复起服务
  - 启动：runtime\\python.exe -m uvicorn app.main:app（CREATE_NO_WINDOW，日志落 logs/server-<日期>.log）
  - --stop：按锁 PID 结束进程（先 taskkill 再 /F），删锁，退出码 0

环境变量 LS_LAUNCHER_NO_BROWSER=1 时不开浏览器（自动化测试用）。
"""
import logging
import os
import socket
import subprocess
import sys
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
PORT_DEFAULT = 8000
CREATE_NO_WINDOW = 0x08000000

# 首次播种（任务47）：schema 与 server/app/store.py 同步；仅空库执行，绝不覆盖用户数据
SEED_SCHEMA = """
CREATE TABLE IF NOT EXISTS cameras(
  id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT, source TEXT, type TEXT, status TEXT DEFAULT 'stopped');
CREATE TABLE IF NOT EXISTS zones(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, name TEXT, polygon TEXT);
CREATE TABLE IF NOT EXISTS rules(
  id INTEGER PRIMARY KEY AUTOINCREMENT, camera_id INTEGER, zone_id INTEGER,
  behavior TEXT, level INTEGER, cooldown_s INTEGER,
  time_start TEXT, time_end TEXT, threshold INTEGER, enabled INTEGER);
CREATE TABLE IF NOT EXISTS events(
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts TEXT, camera_id INTEGER, type TEXT,
  level INTEGER, track_id INTEGER, detail TEXT, snapshot TEXT, video TEXT,
  status TEXT DEFAULT 'new', handled_by TEXT, handled_ts TEXT);
CREATE TABLE IF NOT EXISTS face_consents(
  subject TEXT PRIMARY KEY, ts TEXT);
CREATE TABLE IF NOT EXISTS alert_deliveries(
  id INTEGER PRIMARY KEY AUTOINCREMENT, event_id INTEGER, channel TEXT,
  status TEXT DEFAULT 'pending', attempts INTEGER DEFAULT 0,
  last_error TEXT, updated_ts TEXT);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_camera ON events(camera_id);
"""
SEED_CAMERAS = [  # (名称, 演示视频, 区名)
    ("lab-slice", "lab-walk-24076.mp4", "reagent-area"),
    ("ppe-regress", "lab-walk-24076.mp4", "reagent"),
    ("ppe-final", "lab-walk-24076.mp4", "reagent"),
    ("ppe-equip", "lab-equip-21453.mp4", "bench"),
]
SEED_ZONES = {
    "reagent-area": "[[520, 180], [1180, 180], [1180, 620], [520, 620]]",
    "reagent": "[[520, 180], [1180, 180], [1180, 620], [520, 620]]",
    "bench": "[[100, 50], [1180, 50], [1180, 650], [100, 650]]",
}

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
    """首次运行播种（任务47）：cameras 表为空且 media 在 → 播 4 路演示摄像头/区域/规则。
    用户已有数据（cameras 非空）绝不覆盖；media 缺失只警告不阻塞。"""
    import sqlite3
    db_path = DATA / "labsentinel.db"
    db = sqlite3.connect(db_path)
    try:
        db.executescript(SEED_SCHEMA)
        n = db.execute("select count(*) from cameras").fetchone()[0]
        if n:
            log.info("已有 %d 路摄像头，跳过播种", n)
            return
        media_ok = all((MEDIA / fn).exists() for _, fn, _ in SEED_CAMERAS)
        if not media_ok:
            log.warning("media/ 演示视频缺失，跳过播种（不影响启动）")
            return
        for name, fn, zname in SEED_CAMERAS:
            cid = db.execute("insert into cameras(name, source, type) values(?,?,'file')",
                             (name, (MEDIA / fn).as_posix())).lastrowid
            zid = db.execute("insert into zones(camera_id, name, polygon) values(?,?,?)",
                             (cid, zname, SEED_ZONES[zname])).lastrowid
            for behavior, level in (("enter", 2), ("enter_no_ppe", 3)):
                db.execute("insert into rules(camera_id, zone_id, behavior, level, cooldown_s,"
                           " time_start, time_end, threshold, enabled) values(?,?,?,?,5,'00:00','23:59',0,1)",
                           (cid, zid, behavior, level))
        db.commit()
        log.info("首次播种完成：4 路演示摄像头 + 区域/规则（media=%s）", MEDIA)
    finally:
        db.close()


def first_run(cfg: dict) -> None:
    """首次运行：建数据目录；从模板生成 config.env，模板相对路径改便携包绝对路径。"""
    for d in (DATA, DATA / "snapshots", DATA / "videos", DATA / "annotation_pool", LOGS):
        d.mkdir(parents=True, exist_ok=True)
    seed_if_empty()
    if CONFIG.exists():
        return
    text = TEMPLATE.read_text(encoding="utf-8") if TEMPLATE.exists() else ""
    model_abs = (APP_ROOT / "models" / "yolo11n_ppe.onnx").as_posix()
    text = "\n".join(ln for ln in text.splitlines() if not ln.strip().startswith("LS_PPE_MODEL="))
    text += f"\nLS_PPE_MODEL={model_abs}\nport={cfg.get('port', PORT_DEFAULT)}\n"
    CONFIG.write_text(text, encoding="utf-8")
    log.info("首次运行：config.env 已生成 %s", CONFIG)


def pick_port(cfg: dict) -> int:
    want = int(cfg.get("port", PORT_DEFAULT))

    def free(p):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
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
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/system", timeout=timeout) as r:
            return r.status == 200
    except OSError:
        return False


def read_lock() -> tuple[int, int] | None:
    try:
        pid, port = LOCK.read_text(encoding="ascii").split()
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
           "PYTHONUTF8": "1"}
    for k, v in cfg.items():          # config.env 里的 LS_* 透传（用户可加告警渠道等）
        if k.startswith("LS_"):
            env[k] = v
    server_log = LOGS / f"server-{datetime.now():%Y%m%d}.log"
    with open(server_log, "ab") as lf:
        lf.write(f"\n===== launcher 启动 {datetime.now():%Y-%m-%d %H:%M:%S} port={port} =====\n".encode())
        proc = subprocess.Popen(
            [str(RUNTIME / "python.exe"), "-m", "uvicorn", "app.main:app", "--port", str(port)],
            cwd=str(SERVER), env=env, stdout=lf, stderr=subprocess.STDOUT,
            creationflags=CREATE_NO_WINDOW)
    log.info("uvicorn pid=%d port=%d 日志=%s", proc.pid, port, server_log)
    return proc.pid


def open_browser(port: int) -> None:
    if os.environ.get("LS_LAUNCHER_NO_BROWSER"):
        log.info("LS_LAUNCHER_NO_BROWSER=1，跳过打开浏览器")
        return
    webbrowser.open(f"http://127.0.0.1:{port}/")


def do_start() -> int:
    setup_logging()
    log.info("===== launcher start =====")
    if not (RUNTIME / "python.exe").exists():
        log.error("找不到 runtime\\python.exe：%s", RUNTIME)
        return 1
    cfg = read_config()
    first_run(cfg)
    cfg = read_config()

    lock = read_lock()
    if lock and pid_alive(lock[0]):
        if health(lock[1]):
            log.info("服务已在运行 pid=%d port=%d，直接打开浏览器", *lock)
            open_browser(lock[1])
            return 0
        log.info("锁内 pid=%d 存活但健康检查失败，视为陈旧锁，清理后重启", lock[0])
        LOCK.unlink(missing_ok=True)
    elif lock:
        LOCK.unlink(missing_ok=True)

    port = pick_port(cfg)
    write_config_port(port)
    pid = start_server(port, cfg)
    try:
        LOCK.write_text(f"{pid} {port}", encoding="ascii")
    except OSError as e:
        log.warning("锁写入失败（重试一次）: %s", e)
        time.sleep(0.5)
        LOCK.write_text(f"{pid} {port}", encoding="ascii")

    t0 = time.monotonic()
    while time.monotonic() - t0 < 30:
        if health(port):
            log.info("服务就绪（%.1fs）http://127.0.0.1:%d/", time.monotonic() - t0, port)
            open_browser(port)
            return 0
        time.sleep(1)
    log.error("30s 内未就绪，请查看 %s", LOGS / f"server-{datetime.now():%Y%m%d}.log")
    try:
        import ctypes
        ctypes.windll.user32.MessageBoxW(
            None, f"服务 30 秒内未就绪。\n日志：{LOGS}", "LabSentinel 启动失败", 0x10)
    except Exception:  # noqa: BLE001
        pass
    return 1


def do_stop() -> int:
    setup_logging()
    lock = read_lock()
    if not lock:
        print("未发现运行中的 LabSentinel（无锁文件）。")
        return 0
    pid, port = lock
    subprocess.run(["taskkill", "/PID", str(pid)], capture_output=True)
    time.sleep(2)
    subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
    LOCK.unlink(missing_ok=True)
    log.info("已停止 pid=%d port=%d", pid, port)
    print(f"已停止 LabSentinel（pid={pid}, port={port}）。")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(do_stop() if "--stop" in sys.argv else do_start())
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
