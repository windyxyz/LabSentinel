"""T-107（S-06）：告警密钥加密存储。

DB `app_settings` 中敏感字段只存密文 `enc:v1:<b64>`；Windows 用 DPAPI
（用户范围，crypt32.CryptProtectData），POSIX 用 data/secret.key（0600）
+ cryptography.Fernet；读取兼容历史明文（保存时自动升级）。
备份 zip 内只有密文——换机器解密失败时渠道显示"需要重新填写"，绝不崩溃。
"""
from __future__ import annotations

import base64
import logging
import os
import secrets
from pathlib import Path

log = logging.getLogger("app.secretstore")

_PREFIX = "enc:v1:"


def _dpapi_protect(data: bytes) -> bytes | None:
    """Windows DPAPI（CryptProtectData，用户范围，UI 禁止）。"""
    if os.name != "nt":
        return None
    import ctypes
    from ctypes import wintypes

    class DATA_BLOB(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_char))]

    def _blob(b: bytes) -> DATA_BLOB:
        buf = ctypes.create_string_buffer(b, len(b))
        return DATA_BLOB(len(b), ctypes.cast(buf, ctypes.POINTER(ctypes.c_char))), buf

    crypt32 = ctypes.windll.crypt32
    crypt32.CryptProtectData.argtypes = [
        ctypes.POINTER(DATA_BLOB), wintypes.LPCWSTR, ctypes.c_void_p,
        ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DATA_BLOB)]
    crypt32.CryptProtectData.restype = wintypes.BOOL
    crypt32.CryptUnprotectData.argtypes = [
        ctypes.POINTER(DATA_BLOB), ctypes.c_void_p, ctypes.c_void_p,
        ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DATA_BLOB)]
    crypt32.CryptUnprotectData.restype = wintypes.BOOL

    src, keep_src = _blob(data)
    dst, keep_dst = _blob(b"")
    # CRYPTPROTECT_UI_FORBIDDEN = 0x1
    ok = crypt32.CryptProtectData(ctypes.byref(src), None, None, None, None, 0x1,
                                  ctypes.byref(dst))
    if not ok or not dst.cbData:
        return None
    out = ctypes.string_at(dst.pbData, dst.cbData)
    ctypes.windll.kernel32.LocalFree(dst.pbData)
    # keep 引用防止 GC 提前回收输入缓冲
    _ = (keep_src, keep_dst)
    return out


def _dpapi_unprotect(data: bytes) -> bytes | None:
    if os.name != "nt":
        return None
    import ctypes
    from ctypes import wintypes

    class DATA_BLOB(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_char))]

    def _blob(b: bytes) -> DATA_BLOB:
        buf = ctypes.create_string_buffer(b, len(b))
        return DATA_BLOB(len(b), ctypes.cast(buf, ctypes.POINTER(ctypes.c_char))), buf

    crypt32 = ctypes.windll.crypt32
    crypt32.CryptUnprotectData.argtypes = [
        ctypes.POINTER(DATA_BLOB), ctypes.c_void_p, ctypes.c_void_p,
        ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DATA_BLOB)]
    crypt32.CryptUnprotectData.restype = wintypes.BOOL
    src, keep_src = _blob(data)
    dst, keep_dst = _blob(b"")
    ok = crypt32.CryptUnprotectData(ctypes.byref(src), None, None, None, None, 0x1,
                                    ctypes.byref(dst))
    if not ok or not dst.cbData:
        return None
    out = ctypes.string_at(dst.pbData, dst.cbData)
    ctypes.windll.kernel32.LocalFree(dst.pbData)
    _ = (keep_src, keep_dst)
    return out


def _fernet(data_dir: Path):
    """POSIX：secret.key 不存在则生成（0600）。返回 Fernet 或 None（依赖缺失）。"""
    try:
        from cryptography.fernet import Fernet
    except ImportError:
        return None
    p = Path(data_dir) / "secret.key"
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(Fernet.generate_key())
        if os.name == "posix":
            p.chmod(0o600)
    return Fernet(p.read_bytes())


def protect(value: str, data_dir: Path) -> str:
    """加密；失败降级明文（不阻断配置保存，日志告警）。"""
    raw = value.encode("utf-8")
    blob: bytes | None = None
    if os.name == "nt":
        blob = _dpapi_protect(raw)
    if blob is None:
        f = _fernet(data_dir)
        if f is not None:
            blob = f.encrypt(raw)
    if blob is None:
        log.warning("[secretstore] 无可用加密后端，敏感值以明文保存")
        return value
    return _PREFIX + base64.urlsafe_b64encode(blob).decode("ascii")


def unprotect(value: str, data_dir: Path) -> str:
    """解密 `enc:v1:`；历史明文原样返回；解密失败返回空串（渠道显示需重填）。"""
    if not value.startswith(_PREFIX):
        return value
    blob = base64.urlsafe_b64decode(value[len(_PREFIX):].encode("ascii"))
    out: bytes | None = None
    if os.name == "nt":
        out = _dpapi_unprotect(blob)
    if out is None:
        f = _fernet(data_dir)
        if f is not None:
            try:
                out = f.decrypt(blob)
            except Exception:  # noqa: BLE001
                out = None
    if out is None:
        log.warning("[secretstore] 密文无法解密（换机器/密钥丢失？）——渠道需重新填写")
        return ""
    return out.decode("utf-8")
