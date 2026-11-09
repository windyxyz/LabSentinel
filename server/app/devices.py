"""T-620（Q-11）：设备配对与三级令牌。

- 管理令牌：全部权限（现有 LS_ADMIN_TOKEN）
- 只读令牌：查看（现有 LS_READ_TOKEN）
- 设备令牌：查看 + 事件处置（acked/false_positive），按设备签发/可吊销

配对流程：管理员 POST /api/pairing（响应含一次性 6 位码 + 5 分钟有效期）→ 客户端
POST /api/pairing/claim {code, device_name, platform} → 返回设备令牌（只显示一次，
服务端只存 sha256 hash）。设置页可吊销。
"""
import hashlib
import secrets
import threading
import time
from datetime import datetime

_PAIR_TTL_S = 300
_TOUCH_INTERVAL_S = 60  # R2-103: do not write SQLite on every read request
_pair_lock = threading.Lock()
#: code -> (expire_ts, created_by)
_pending: dict[str, float] = {}
#: device_id -> last touch_ts（进程内节流，重启后自然重置）
_last_touch: dict[int, float] = {}


def _hash(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def new_pairing_code() -> str:
    """生成一次性 6 位数字码（5 分钟有效；同码覆盖旧码）。"""
    code = f"{secrets.randbelow(1_000_000):06d}"
    with _pair_lock:
        now = time.time()
        for k in [k for k, v in _pending.items() if v < now]:
            _pending.pop(k, None)
        _pending[code] = now + _PAIR_TTL_S
    return code


def claim(store, code: str, device_name: str, platform: str = "") -> str | None:
    """用配对码换设备令牌；成功后码作废。返回令牌明文（仅此一次）或 None。"""
    with _pair_lock:
        exp = _pending.pop(code, None)
        if exp is None or exp < time.time():
            return None
    token = secrets.token_urlsafe(32)
    store.add_device(device_name or "device", platform, _hash(token))
    return token


def verify(store, token: str) -> dict | None:
    """设备令牌校验：返回设备行（未吊销）或 None；顺带刷新 last_seen。

    R2-103: _read_token_ok now calls this on every protected read, so throttle
    touch_device writes to at most once per device per _TOUCH_INTERVAL_S.
    """
    if not token:
        return None
    row = store.device_by_hash(_hash(token))
    if row is None or row.get("revoked"):
        return None
    did, now = row["id"], time.time()
    with _pair_lock:
        if now - _last_touch.get(did, 0.0) < _TOUCH_INTERVAL_S:
            return row
        _last_touch[did] = now
    try:
        store.touch_device(did)
    except Exception:  # noqa: BLE001
        pass
    return row


def can_triage(token: str, store) -> bool:
    """设备令牌允许事件处置（acked/false_positive），不允许改配置。"""
    return verify(store, token) is not None


def revoke(store, device_id: int) -> bool:
    return store.revoke_device(device_id)


def list_devices(store) -> list[dict]:
    """设备列表（不含 token_hash）。"""
    return [{"id": r["id"], "name": r["name"], "platform": r.get("platform", ""),
             "created_ts": r.get("created_ts"), "last_seen_ts": r.get("last_seen_ts"),
             "revoked": bool(r.get("revoked"))} for r in store.devices()]
