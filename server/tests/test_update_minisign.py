"""R2-107/N-13 回归：更新清单验签按 minisign 格式实现。

minisign .minisig 第 2 行 = base64(alg[2] + key_id[8] + sig[64])：
- `Ed`：直接对 body 签名；
- `ED`：对 BLAKE2b-512(body) 签名（minisign ≥0.10 默认）。
旧实现取最后一行（全局签名）且不做预哈希，合法签名也会被拒。
"""
import base64
import hashlib
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from cryptography.hazmat.primitives import serialization  # noqa: E402
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402

from app import update_check as uc  # noqa: E402

BODY = b'{"version":"0.2.1","notes":"ok"}'
KEY_ID = bytes.fromhex("0102030405060708")


def _minisign_pub(key, key_id=KEY_ID) -> str:
    raw = key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw)
    blob = b"Ed" + key_id + raw
    return "untrusted comment: minisign public key\n" + base64.b64encode(blob).decode() + "\n"


def _minisig(key, body, alg=b"Ed", key_id=KEY_ID, tamper=False) -> str:
    msg = hashlib.blake2b(body, digest_size=64).digest() if alg == b"ED" else body
    sig = key.sign(msg)
    if tamper:
        sig = bytes([sig[0] ^ 1]) + sig[1:]
    blob = alg + key_id + sig
    lines = [
        "untrusted comment: signature from minisign secret key",
        base64.b64encode(blob).decode(),
        "trusted comment: test",
        base64.b64encode(key.sign(sig + b"trusted comment: test")).decode(),
    ]
    return "\n".join(lines) + "\n"


class _FakeResponse:
    def __init__(self, text):
        self.status_code = 200
        self.text = text


class _FakeClient:
    def __init__(self, text):
        self._text = text

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False

    def get(self, path):
        return _FakeResponse(self._text)


@pytest.mark.parametrize("alg", [b"Ed", b"ED"])
def test_valid_minisign_signature(tmp_path, monkeypatch, alg):
    key = Ed25519PrivateKey.generate()
    pub = tmp_path / "update.pub"
    pub.write_text(_minisign_pub(key), encoding="ascii")
    monkeypatch.setattr(uc, "__file__", str(tmp_path / "update_check.py"))
    # keys/update.pub 与伪造的 __file__ 同目录
    keys = tmp_path / "keys"
    keys.mkdir()
    (keys / "update.pub").write_text(_minisign_pub(key), encoding="ascii")
    monkeypatch.setattr(uc.httpx, "Client", lambda **kw: _FakeClient(_minisig(key, BODY, alg)))
    uc._verify_signature("https://example.com/latest.json", BODY, 5.0)


def test_tampered_body_rejected(tmp_path, monkeypatch):
    key = Ed25519PrivateKey.generate()
    keys = tmp_path / "keys"
    keys.mkdir()
    (keys / "update.pub").write_text(_minisign_pub(key), encoding="ascii")
    monkeypatch.setattr(uc, "__file__", str(tmp_path / "update_check.py"))
    monkeypatch.setattr(uc.httpx, "Client", lambda **kw: _FakeClient(_minisig(key, BODY)))
    with pytest.raises(uc.UpdateCheckError, match="签名"):
        uc._verify_signature("https://example.com/latest.json", BODY + b"x", 5.0)


def test_key_id_mismatch_rejected(tmp_path, monkeypatch):
    key = Ed25519PrivateKey.generate()
    keys = tmp_path / "keys"
    keys.mkdir()
    (keys / "update.pub").write_text(_minisign_pub(key), encoding="ascii")
    monkeypatch.setattr(uc, "__file__", str(tmp_path / "update_check.py"))
    wrong = bytes.fromhex("ffffffffffffffff")
    monkeypatch.setattr(uc.httpx, "Client",
                        lambda **kw: _FakeClient(_minisig(key, BODY, key_id=wrong)))
    with pytest.raises(uc.UpdateCheckError, match="key_id"):
        uc._verify_signature("https://example.com/latest.json", BODY, 5.0)


def test_tampered_sig_rejected(tmp_path, monkeypatch):
    key = Ed25519PrivateKey.generate()
    keys = tmp_path / "keys"
    keys.mkdir()
    (keys / "update.pub").write_text(_minisign_pub(key), encoding="ascii")
    monkeypatch.setattr(uc, "__file__", str(tmp_path / "update_check.py"))
    monkeypatch.setattr(uc.httpx, "Client",
                        lambda **kw: _FakeClient(_minisig(key, BODY, tamper=True)))
    with pytest.raises(uc.UpdateCheckError, match="签名"):
        uc._verify_signature("https://example.com/latest.json", BODY, 5.0)


def test_pub_missing_skips(tmp_path, monkeypatch):
    monkeypatch.setattr(uc, "__file__", str(tmp_path / "update_check.py"))
    uc._verify_signature("https://example.com/latest.json", BODY, 5.0)  # 不抛
