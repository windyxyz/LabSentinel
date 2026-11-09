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


def _parse_minisign_pub(text: str) -> tuple[bytes, bytes]:
    """返回 (key_id, public_key_bytes)。minisign 公钥第二行 base64：
    alg(2B) + key_id(8B) + pk(32B)；也兼容纯 64 hex（key_id 为空，跳过比对）。
    """
    import base64
    lines = [ln.strip() for ln in text.strip().splitlines() if ln.strip()]
    if len(lines) >= 2 and not all(c in "0123456789abcdefABCDEF" for c in lines[-1]):
        blob = base64.b64decode(lines[-1])
        if len(blob) < 42 or blob[:2] not in (b"Ed", b"ED"):
            raise UpdateCheckError("minisign 公钥格式无效（alg 不是 Ed/ED）")
        return blob[2:10], blob[10:42]
    raw = lines[-1]
    if len(raw) == 64 and all(c in "0123456789abcdefABCDEF" for c in raw):
        return b"", bytes.fromhex(raw)  # 旧格式：无 key_id
    raise UpdateCheckError("更新公钥格式无效（需要 minisign 公钥或 64 hex）")


def _verify_signature(manifest_url: str, body: bytes, timeout: float) -> None:
    """R2-107/N-13：按 minisign 格式验签（公钥存在才启用）。

    .minisig 结构：第 1 行注释；第 2 行 base64 = alg(2B: Ed/ED) + key_id(8B)
    + signature(64B)；第 3 行 trusted comment；第 4 行全局签名。
    旧实现取**最后一行**（全局签名）当文件签名且直接 verify(body)，
    放入合法公钥后仍会被拒。ED（minisign ≥0.10 默认）对 body 先做 BLAKE2b-512
    预哈希再验签；Ed 直接验 body。公钥 key_id 必须匹配。
    """
    import base64
    import hashlib
    from pathlib import Path

    from cryptography.exceptions import InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

    pub = Path(__file__).resolve().parent / "keys" / "update.pub"
    if not pub.exists():
        return  # 渐进式：签名体系部署前清单照常工作
    try:
        key_id, key_bytes = _parse_minisign_pub(pub.read_text("ascii"))
        key = Ed25519PublicKey.from_public_bytes(key_bytes)
    except UpdateCheckError:
        raise
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
        lines = r.text.strip().splitlines()
        if len(lines) < 2:
            raise UpdateCheckError("minisign 签名文件缺第 2 行签名")
        blob = base64.b64decode(lines[1].strip())
        if len(blob) < 74:
            raise UpdateCheckError("minisign 签名长度不足（需要 alg+key_id+sig）")
        alg, sig_key_id, sig = blob[:2], blob[2:10], blob[10:74]
        if alg not in (b"Ed", b"ED"):
            raise UpdateCheckError("minisign 签名算法不是 Ed/ED")
        if key_id and key_id != sig_key_id:
            raise UpdateCheckError("minisign 签名公钥 key_id 不匹配")
        msg = (hashlib.blake2b(body, digest_size=64).digest()
               if alg == b"ED" else body)
        key.verify(sig, msg)
    except UpdateCheckError:
        raise
    except InvalidSignature as e:
        raise UpdateCheckError("更新清单签名校验失败") from e
    except (ValueError, TypeError) as e:
        raise UpdateCheckError(f"minisign 签名解析失败: {type(e).__name__}") from e
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
