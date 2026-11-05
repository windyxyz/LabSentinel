"""版本更新检查（任务117）：只 GET 静态清单并提示，绝不下载/安装/执行任何 URL。

安全边界：
- 清单地址默认仅 https（自建局域网 http 需显式 LS_UPDATE_ALLOW_HTTP=1）；
- 不跟随重定向（follow_redirects=False）——重定向到任意 http 地址是信息/下载面；
- 只发一个 GET，不带任何本机信息（无 UA 外字段、无 POST 体）；
- 结果 1h 缓存；任何失败转 error 字段，不 500。
比较口径：version 字符串与当前版本**不等即提示**（不做 semver 排序——便携版版本形如 2026.11，
日期语义，排序无必要）。
"""
import ipaddress
import socket
import threading
import time
from urllib.parse import urlparse

import httpx

CACHE_TTL_S = 3600.0

_cache = {"ts": 0.0, "result": None}
_lock = threading.Lock()


class UpdateCheckError(Exception):
    pass


def _assert_public_host(manifest_url: str) -> None:
    """SSRF 防护（T-110 前置）：清单主机必须解析为公网地址，拒绝回环/私网/链路本地/
    保留段（含 https 指向内网 IP 的形态）；仅允许公网清单源。"""
    host = urlparse(manifest_url).hostname
    if not host:
        raise UpdateCheckError("清单地址缺主机名")
    try:
        infos = socket.getaddrinfo(host, None)
    except socket.gaierror as e:
        raise UpdateCheckError(f"清单主机解析失败：{type(e).__name__}") from e
    for info in infos:
        ip = ipaddress.ip_address(info[4][0])
        if not ip.is_global:
            raise UpdateCheckError("清单主机不是公网地址（拒绝访问内网/回环）")


def fetch_manifest(manifest_url: str, allow_http: bool, timeout: float) -> dict:
    """GET 并校验清单。失败抛 UpdateCheckError（信息可读、不含 URL 凭证类内容）。
    请求形态：base_url Client + 相对路径（host 已过 scheme/公网校验）。"""
    parts = urlparse(manifest_url)
    scheme_ok = parts.scheme == "https" or (parts.scheme == "http" and allow_http)
    if not scheme_ok or not parts.netloc:
        raise UpdateCheckError("清单地址必须为 https（或显式开启 LS_UPDATE_ALLOW_HTTP）")
    if not allow_http:
        _assert_public_host(manifest_url)
    base = f"{parts.scheme}://{parts.netloc}"
    path = parts.path or "/"
    if parts.query:
        path += "?" + parts.query
    try:
        with httpx.Client(base_url=base, timeout=timeout, follow_redirects=False) as client:
            r = client.get(path)
    except httpx.HTTPError as e:
        raise UpdateCheckError(f"清单请求失败：{type(e).__name__}") from e
    if r.status_code != 200:
        raise UpdateCheckError(f"清单返回 HTTP {r.status_code}")
    try:
        data = r.json()
    except ValueError as e:
        raise UpdateCheckError("清单不是合法 JSON") from e
    if not isinstance(data, dict) or not isinstance(data.get("version"), str):
        raise UpdateCheckError("清单缺 version 字段（或非字符串）")
    return data


def check_update(manifest_url: str, current: str, allow_http: bool = False,
                 timeout: float = 5.0, refresh: bool = False) -> dict:
    """检查更新（1h 缓存）。未配置返回 enabled=false；失败返回 error，不抛。"""
    with _lock:
        if not refresh and _cache["result"] is not None \
                and time.time() - _cache["ts"] < CACHE_TTL_S:
            return dict(_cache["result"])
    result = {"enabled": bool(manifest_url), "current": current,
              "update_available": False, "latest": None, "url": None,
              "sha256": None, "notes": None, "published_at": None,
              "channel": None, "error": ""}
    if manifest_url:
        try:
            data = fetch_manifest(manifest_url, allow_http, timeout)
        except UpdateCheckError as e:
            result["error"] = str(e)  # 错误不缓存：下次调用可重试（网络瞬时失败不粘住 1h）
        else:
            result["latest"] = data["version"]
            result["update_available"] = data["version"] != current
            for k in ("url", "sha256", "notes", "published_at", "channel"):
                v = data.get(k)
                result[k] = v if isinstance(v, str) else None
            with _lock:
                _cache.update({"ts": time.time(), "result": dict(result)})
    return result
