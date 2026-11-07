"""单元测试：任务113 局域网发现助手（样例 XML 解析 / probe mock / API；不发真组播）。。
运行: pytest tests/test_discovery.py -q"""
import socket
import subprocess

import pytest
from fastapi.testclient import TestClient

from app import api as api_mod
from app import discovery
from app.main import app

SAMPLE_1 = """<?xml version="1.0" encoding="UTF-8"?>
<SOAP-ENV:Envelope xmlns:SOAP-ENV="http://www.w3.org/2003/05/soap-envelope"
   xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery">
 <SOAP-ENV:Body>
  <d:ProbeMatches>
   <d:ProbeMatch>
    <d:Scopes>onvif://www.onvif.org/name/IPC-123 onvif://www.onvif.org/Profile/Streaming</d:Scopes>
    <d:XAddrs>http://192.168.1.64:80/onvif/device_service</d:XAddrs>
   </d:ProbeMatch>
  </d:ProbeMatches>
 </SOAP-ENV:Body>
</SOAP-ENV:Envelope>"""

SAMPLE_2 = SAMPLE_1.replace("192.168.1.64", "10.0.0.23").replace(
    "IPC-123", "NVR-2")


# ---------- is_lan 白名单 ----------

def test_is_lan_whitelist():
    assert discovery.is_lan("192.168.1.64")
    assert discovery.is_lan("10.0.0.23")
    assert discovery.is_lan("172.20.1.5")
    assert discovery.is_lan("169.254.9.9")
    assert not discovery.is_lan("8.8.8.8")          # 公网
    assert not discovery.is_lan("127.0.0.1")        # 回环
    assert not discovery.is_lan("172.32.0.1")       # 172.16/12 之外
    assert not discovery.is_lan("not-an-ip")


# ---------- XML 解析 ----------

def test_parse_single_device():
    out = discovery._parse_probe_response(SAMPLE_1)
    assert len(out) == 1
    assert out[0]["ip"] == "192.168.1.64"
    assert out[0]["name"] == "IPC-123"
    assert "onvif" in out[0]["xaddr"]


def test_parse_multi_and_dedup_by_ip():
    tail = SAMPLE_2.split("<SOAP-ENV:Body>", 1)[1].replace("</SOAP-ENV:Envelope>", "")
    combined = SAMPLE_1.replace("</SOAP-ENV:Body>", tail)  # Body 内两个 ProbeMatch
    out = discovery._parse_probe_response(combined)
    assert {d["ip"] for d in out} == {"192.168.1.64", "10.0.0.23"}


def test_parse_malformed_and_xxe_rejected():
    assert discovery._parse_probe_response("") == []
    assert discovery._parse_probe_response("<not-xml") == []
    assert discovery._parse_probe_response("这不是 xml") == []
    xxe = ('<?xml version="1.0"?><!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///c:/windows/win.ini">]>'
           "<a>&xxe;</a>")
    assert discovery._parse_probe_response(xxe) == []  # DOCTYPE 直接拒绝


def test_ws_discover_public_ip_filtered(monkeypatch):
    """应答里带公网地址 → 被过滤掉（绝不外泄到结果）。"""
    class FakeSock:
        def __init__(self):
            self.n = 0

        def setsockopt(self, *a):
            pass

        def settimeout(self, t):
            pass

        def sendto(self, *a):
            pass

        def recvfrom(self, n):
            self.n += 1
            if self.n > 1:
                raise socket.timeout  # 只回一包，然后模拟超时收尾
            return SAMPLE_1.replace("192.168.1.64", "8.8.8.8").encode(), ("1.2.3.4", 3702)

        def close(self):
            pass

    monkeypatch.setattr(discovery.socket, "socket", lambda *a, **k: FakeSock())
    assert discovery.ws_discover(timeout=0.1) == []  # 公网 IP 被过滤 → 空


# ---------- probe_host（mock socket/ffprobe） ----------

def test_probe_host_port_open_with_ffprobe(monkeypatch):
    monkeypatch.setattr(discovery.socket, "create_connection",
                        lambda addr, timeout: socket.socket())
    monkeypatch.setattr(discovery, "_ffprobe_stream",
                        lambda url: {"codec": "h264", "width": 1280, "height": 720,
                                     "fps": 15.0})
    r = discovery.probe_host("192.168.1.64")
    assert r["port_open"] is True and r["error"] == ""
    assert r["ffprobe"] == {"codec": "h264", "width": 1280, "height": 720, "fps": 15.0}


def test_probe_host_refused_readable_error(monkeypatch):
    def refused(addr, timeout):
        raise ConnectionRefusedError(10061, "连接被拒绝")
    refused.strerror = None
    monkeypatch.setattr(discovery.socket, "create_connection", refused)
    r = discovery.probe_host("192.168.1.64", port=554)
    assert r["port_open"] is False
    assert "554" in r["error"] and "连接失败" in r["error"]


def test_probe_host_timeout(monkeypatch):
    def slow(addr, timeout):
        raise TimeoutError()
    slow.strerror = None
    monkeypatch.setattr(discovery.socket, "create_connection", slow)
    r = discovery.probe_host("192.168.1.64")
    assert "超时" in r["error"] or "拒绝" in r["error"]


def test_probe_host_rejects_public_and_unresolvable(monkeypatch):
    monkeypatch.setattr(discovery.socket, "gethostbyname", lambda h: "8.8.8.8")
    r = discovery.probe_host("evil.example.com")
    assert "局域网" in r["error"] and r["port_open"] is False
    monkeypatch.setattr(discovery.socket, "gethostbyname",
                        lambda h: (_ for _ in ()).throw(OSError("nx")))
    r2 = discovery.probe_host("nope.local")
    assert "无法解析" in r2["error"]


def test_probe_host_never_returns_credentials(monkeypatch):
    """带凭证探测：返回体/错误串绝不出现用户名密码。"""
    monkeypatch.setattr(discovery.socket, "create_connection",
                        lambda addr, timeout: socket.socket())

    def fake_ff(url):
        raise AssertionError("不应把 URL 泄露到结果") if "secrets" not in url else None

    seen = {}

    def ff(url):
        seen["url"] = url
        return None

    monkeypatch.setattr(discovery, "_ffprobe_stream", ff)
    r = discovery.probe_host("192.168.1.64", username="admin", password="s3cret!")
    # 凭证可进入探测 URL（ffprobe 需要），但绝不出现在返回体里
    assert "s3cret" not in str(r) and "password" not in str(r)
    assert seen["url"].startswith("rtsp://admin:")  # URL 确实带上了凭证（设计如此）


def test_eval_fraction():
    assert discovery.eval_fraction("30000/1001") == pytest.approx(29.97, abs=0.01)
    assert discovery.eval_fraction("25/1") == 25.0
    assert discovery.eval_fraction("") == 0.0
    assert discovery.eval_fraction("abc") == 0.0  # 非法分数容错返回 0（ffprobe 输出不可信）


# ---------- API ----------

@pytest.fixture()
def client():
    with TestClient(app) as tc:
        yield tc


def test_discovery_api_empty_hint_200(client, monkeypatch):
    monkeypatch.setattr(discovery, "get_devices", lambda timeout, refresh=False: [])
    r = client.get("/api/discovery/cameras")
    assert r.status_code == 200
    body = r.json()
    assert body["devices"] == [] and "未发现设备" in body["error"]
    assert isinstance(body["lan"], list)


def test_discovery_api_devices_and_refresh(client, monkeypatch):
    calls = []

    def fake_get(timeout, refresh=False):
        calls.append(refresh)
        return [{"ip": "192.168.1.64", "xaddr": "http://192.168.1.64:80/onvif",
                 "name": "IPC", "scopes": ""}]

    monkeypatch.setattr(api_mod.discovery, "get_devices", fake_get)
    body = client.get("/api/discovery/cameras").json()
    assert body["devices"][0]["ip"] == "192.168.1.64" and body["error"] == ""
    client.get("/api/discovery/cameras?refresh=1")
    assert calls == [False, True]


def test_discovery_probe_api(client, monkeypatch):
    monkeypatch.setattr(discovery.socket, "gethostbyname", lambda h: "192.168.1.64")
    monkeypatch.setattr(discovery.socket, "create_connection",
                        lambda addr, timeout: socket.socket())
    monkeypatch.setattr(discovery, "_ffprobe_stream", lambda url: None)
    r = client.post("/api/discovery/probe", json={"host": "192.168.1.64"})
    assert r.status_code == 200 and r.json()["port_open"] is True
    # T-505/B-23：DiscoveryProbeBody → host 空/端口越界 422
    assert client.post("/api/discovery/probe", json={"host": ""}).status_code == 422
    assert client.post("/api/discovery/probe",
                       json={"host": "h", "port": 70000}).status_code == 422


def test_discovery_auth(client, monkeypatch):
    monkeypatch.setattr(api_mod.settings, "admin_token", "sec")
    monkeypatch.setattr(api_mod.settings, "auth_read_protected", False)
    # require_admin：无令牌 401（即使读保护未开，admin 端点也要令牌）
    assert client.get("/api/discovery/cameras").status_code in (401, 403)
    assert client.get("/api/discovery/cameras",
                      headers={"X-Admin-Token": "sec"}).status_code == 200
    assert client.post("/api/discovery/probe", json={"host": "192.168.1.1"}
                       ).status_code in (401, 403)
    monkeypatch.setattr(api_mod.settings, "admin_token", "")
