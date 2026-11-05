"""T-303（PF-03）：进程内配置缓存（zones/rules/camera 名）。

热路径每帧 store.zones()/rules()/is_holiday() 都新建连接+PRAGMA；改为按
Store.config_version 判失效后重建（写方法末尾 _bump()）。缓存挂在 Store 实例上
（生命周期绑定，避免 CPython 复用 id 造成跨实例串味），版本号参与 key。
"""
import threading

_lock = threading.Lock()


def _cache(store) -> dict:
    """取/建 Store 实例私有缓存（不污染 Store 类定义，测试 tmp DB 天然隔离）。"""
    with _lock:
        c = getattr(store, "_config_cache", None)
        if c is None:
            c = {}
            store._config_cache = c
        return c


def _snapshot(store, name: str, version: int, loader):
    c = _cache(store)
    stamp = (version,)
    with _lock:
        if c.get(name, (None, None))[0] == stamp:
            return c[name][1]
        data = loader()
        c[name] = (stamp, data)
        return data


def zones(store, camera_id: int) -> list[dict]:
    def load() -> dict:
        grouped: dict[int, list[dict]] = {}
        for z in store.zones():
            grouped.setdefault(z["camera_id"], []).append(z)
        return grouped
    return _snapshot(store, "zones", store.config_version(), load).get(camera_id, [])


def rules(store, camera_id: int | None = None) -> list[dict]:
    all_rules = _snapshot(store, "rules", store.config_version(), lambda: store.rules())
    if camera_id is None:
        return all_rules
    return [r for r in all_rules if r["camera_id"] == camera_id]


def camera_name(store, cid: int) -> str | None:
    names = _snapshot(store, "cameras", store.config_version(),
                      lambda: {c["id"]: c["name"] for c in store.cameras()})
    return names.get(cid)


def invalidate(store) -> None:
    """版本号回绕/Store 重建时手动失效（测试与迁移路径用）。"""
    c = _cache(store)
    with _lock:
        c.clear()
