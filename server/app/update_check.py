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
    body = r.content
    try:
        data = r.json()
    except ValueError as e:
        raise UpdateCheckError("清单不是合法 JSON") from e
    if not isinstance(data, dict) or not isinstance(data.get("version"), str):
        raise UpdateCheckError("清单缺 version 字段（或非字符串）")
    return data, body


def _manifest_sig_url(manifest_url: str) -> str:
    """清单签名侧车：latest.json → latest.json.minisig（T-110）。"""
    return manifest_url.rstrip("/") + ".minisig"


def _verify_signature(manifest_url: str, body: bytes, timeout: float) -> None:
    """Ed25519 验签（公钥 app/keys/update.pub 存在才启用；格式 minisign 或原始 64 字节 hex）。
    公钥缺失时跳过（渐进式：签名体系部署前清单照常工作，T-408 生成公钥后自动强制）。"""
    from pathlib import Path
    pub = Path(__file__).resolve().parent / "keys" / "update.pub"
    if not pub.exists():
        return
    try:
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
        from cryptography.exceptions import InvalidSignature
        key_raw = pub.read_text("ascii").strip()
        # 支持 minisign pub 第二行 base64（前 10 字节头 + 32 字节密钥）或纯 64 hex
        if " " in key_raw or "\n" in key_raw:
            import base64
            b64 = key_raw.splitlines()[-1].strip()
            blob = base64.b64decode(b64)
            key_bytes = blob[-32:]
        else:
            key_bytes = bytes.fromhex(key_raw)
        key = Ed25519PublicKey.from_public_bytes(key_bytes)
    except Exception as e:  # noqa: BLE001
        raise UpdateCheckError(f"更新公钥无效: {type(e).__name__}") from e
    sig_url = _manifest_sig_url(manifest_url)
    parts = urlparse(sig_url)
    base = f"{parts.scheme}://{parts.netloc}"
    path = parts.path or "/"
    try:
        with httpx.Client(base_url=base, timeout=timeout, follow_redirects=False) as client:
            r = client.get(path)
        if r.status_code != 200:
            raise UpdateCheckError("更新清单签名缺失")
        sig_line = r.text.strip().splitlines()[-1]
        import base64
        sig = base64.b64decode(sig_line)
        key.verify(sig, body)
    except InvalidSignature as e:
        raise UpdateCheckError("更新清单签名校验失败") from e
    except httpx.HTTPError as e:
        raise UpdateCheckError(f"更新签名获取失败：{type(e).__name__}") from e


def _version_gt(latest: str, current: str) -> bool:
    """T-110/Q-05：仅更高版本才提示（packaging 语义；旧 CalVer 2026.10 视为低于首个 SemVer）。"""
    try:
        from packaging.version import Version, InvalidVersion
        return Version(latest) > Version(current)
    except (InvalidVersion, ImportError, TypeError):
        if current.startswith("2026.") and not latest.startswith("2026."):
            return True  # Q-05 一次性兼容：旧 CalVer → 新 SemVer
        return latest != current


def _url_allowed(url: str, hosts: str) -> bool:
    """更新 url 必须 https 且 host 在 LS_UPDATE_URL_HOSTS（空=只要求 https）。"""
    if not isinstance(url, str) or not url.startswith("https://"):
        return False
    from urllib.parse import urlparse as _up
    host = (_up(url).hostname or "").lower()
    allowed = [h.strip().lower() for h in (hosts or "").split(",") if h.strip()]
    return not allowed or host in allowed


def check_update(manifest_url: str, current: str, allow_http: bool = False,
                 timeout: float = 5.0, refresh: bool = False,
                 pubkey_path: str = "", url_hosts: str = "") -> dict:
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
            data, body = fetch_manifest(manifest_url, allow_http, timeout)
            _verify_signature(manifest_url, body, timeout)  # T-110
        except UpdateCheckError as e:
            result["error"] = str(e)  # 错误不缓存：下次调用可重试（网络瞬时失败不粘住 1h）
        else:
            result["latest"] = data["version"]
            result["update_available"] = _version_gt(data["version"], current)  # T-110：只升不降
            for k in ("url", "sha256", "notes", "published_at", "channel"):
                v = data.get(k)
                result[k] = v if isinstance(v, str) else None
            if result["url"] and not _url_allowed(result["url"], url_hosts):  # T-110：url 白名单
                result["url"] = None  # 前端 :href 只渲染 https 且白名单内的链接
            with _lock:
                _cache.update({"ts": time.time(), "result": dict(result)})
    return result
