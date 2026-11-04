"""监控机防休眠（任务123，PowerToys Awake 同源 API）。

服务运行期间用 `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED [| ES_DISPLAY_REQUIRED])`
临时阻止系统睡眠/硬盘休眠——**不改用户电源计划、不写注册表、无需管理员**，退出即恢复。
默认关（LS_KEEP_AWAKE=false）：未经用户同意不改变机器行为。只阻止系统睡眠，
**不阻止用户注销/关机**。

Win32 语义：SetThreadExecutionState 的效果是 **per-thread** 的（ES_CONTINUOUS 保持到
该线程再次调用或退出）。所以首个断言、每 45s 重断言（部分驱动/组策略会重置）、
退出时的解除，全部在同一个专用线程内配对完成（PowerToys Awake 同款做法）；
进程意外退出时 OS 自动清锁，不会把机器锁成永久不睡。
"""
import sys
import threading
import time

ES_CONTINUOUS = 0x80000000
ES_SYSTEM_REQUIRED = 0x00000001
ES_DISPLAY_REQUIRED = 0x00000002

REASSERT_S = 45.0  # 重断言间隔；测试可调小

_state: dict = {"enabled": False, "active": False, "display": False}
_stop = threading.Event()
_first = threading.Event()  # 首次断言已出结果（start 用它等结果，不能 join 常驻线程）
_thread: threading.Thread | None = None
_kernel32 = None  # 测试可注入 fake


def _assert_once(display: bool) -> bool:
    """断言一次执行状态锁（调用线程持有）。返回是否成功。"""
    global _kernel32
    if _kernel32 is None:
        import ctypes
        _kernel32 = ctypes.windll.kernel32
    flags = ES_CONTINUOUS | ES_SYSTEM_REQUIRED | (ES_DISPLAY_REQUIRED if display else 0)
    return bool(_kernel32.SetThreadExecutionState(flags))  # 返回 0 = 失败


def _release() -> None:
    """仅 ES_CONTINUOUS = 清除本线程全部锁定。"""
    if _kernel32 is None:
        return
    _kernel32.SetThreadExecutionState(ES_CONTINUOUS)


def _worker(display: bool) -> None:
    try:
        first_ok = False
        try:
            first_ok = bool(_assert_once(display))
        except Exception as e:  # noqa: BLE001  无桌面会话/权限问题：只告警
            print(f"[keepawake] SetThreadExecutionState 失败: {e!r}")
        if first_ok:
            _state["active"] = True
        else:
            print("[keepawake] SetThreadExecutionState 未生效（返回 0/异常，组策略或驱动限制?）")
        _first.set()  # start 只等"首次断言出结果"这一刻
        if not first_ok:
            return
        while not _stop.wait(REASSERT_S):
            try:
                if not _assert_once(display):
                    _state["active"] = False
                    print("[keepawake] 重新断言失败，防休眠已失效")
                    return
            except Exception as e:  # noqa: BLE001
                _state["active"] = False
                print(f"[keepawake] 重新断言异常: {e!r}")
                return
        _release()  # 与首个断言同线程配对解除
    finally:
        _state["active"] = False


def start(enabled: bool, display: bool = False) -> bool:
    """开启防休眠（幂等）。返回是否 active。任何失败只告警，不影响服务启动。"""
    global _thread
    if enabled and _thread is not None and _thread.is_alive():
        # 已在跑：幂等返回当前状态（不得把 active 重置掉）
        _state.update(enabled=True, display=bool(display))
        return bool(_state["active"])
    _state.update(enabled=bool(enabled), display=bool(display), active=False)
    if not enabled:
        return False
    if sys.platform != "win32":
        print("[keepawake] 非 Windows 平台，防休眠 no-op")
        return False
    if _thread is not None and _thread.is_alive():
        return bool(_state["active"])  # 已在跑
    _stop.clear()
    _first.clear()
    _thread = threading.Thread(target=_worker, args=(bool(display),),
                               name="keep-awake", daemon=True)
    _thread.start()
    if not _first.wait(timeout=3.0):  # 只等首次断言出结果，不 join 常驻循环
        print("[keepawake] 首次断言超时未完成（线程卡住?），按未生效处理")
    return bool(_state["active"])


def stop() -> None:
    """解除防休眠（幂等；未开启/非 Windows 安全 no-op）。"""
    global _thread
    _stop.set()
    t, _thread = _thread, None
    if t is not None:
        t.join(timeout=3.0)  # 解除（ES_CONTINUOUS）由 worker 线程自己在退出前完成
    _state.update(active=False)


def status() -> dict:
    return dict(_state)
