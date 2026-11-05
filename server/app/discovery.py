"""局域网摄像头发现助手（任务113）：WS-Discovery(ONVIF) + RTSP 端口/编码探测。

安全边界（规格约束）：
- 只发一次 UDP 组播 Probe（NetworkVideoTransmitter），只收 timeout 秒内的应答；
- 结果仅保留局域网地址（10/8、172.16/12、192.168/16、169.254/16）——绝不扫公网；
- XML 解析前拒绝 DOCTYPE/ENTITY（XXE/实体爆炸加固），只取 XAddrs/Scopes/name 三个字段；
- probe 结果不含用户凭证；ffprobe 的 stderr 可能回显带凭证的 URL——一律不进返回值/日志；
- 只读探测，不修改摄像头配置，不做 ONVIF 认证/PTZ。
"""
import ipaddress
import re
import shutil
import socket
import subprocess
import threading
import time
import uuid
from pathlib import Path
from urllib.parse import quote, urlparse

from defusedxml import ElementTree as ET  # T-501（S-14）：防 XXE/实体爆炸

WS_MULTICAST_ADDR = "239.255.255.250"
WS_PORT = 3702
CACHE_TTL_S = 30.0
FFPROBE_TIMEOUT_S = 6.0

_LAN_NETS = [ipaddress.ip_network(n) for n in
             ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "169.254.0.0/16")]

_PROBE_TMPL = (
    '<?xml version="1.0" encoding="UTF-8"?>'
    '<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" '
    'xmlns:a="http://schemas.xmlsoap.org/ws/2004/08/addressing">'
    '<s:Header><a:Action s:mustUnderstand="1">'
    'http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</a:Action>'
    '<a:MessageID>urn:uuid:{mid}</a:MessageID>'
    '<a:To s:mustUnderstand="1">urn:schemas-xmlsoap-org:ws:2005:04:discovery</a:To>'
    '</s:Header><s:Body><Probe xmlns="http://schemas.xmlsoap.org/ws/2005/04/discovery">'
    '<d:Types xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" '
    'xmlns:dp0="http://www.onvif.org/ver10/network/wsdl">dp0:NetworkVideoTransmitter'
    '</d:Types></Probe></s:Body></s:Envelope>')

_cache = {"ts": 0.0, "devices": []}
_cache_lock = threading.Lock()


def is_lan(ip: str) -> bool:
    """仅允许 10/8、172.16/12、192.168/16、169.254/16（规格白名单，回环/公网一律拒绝）。"""
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return False
    return any(a in net for net in _LAN_NETS)


def _parse_probe_response(text: str) -> list[dict]:
    """解析一个 WS-Discovery 应答 → [{ip, xaddr, name, scopes}]（纯函数，畸形输入返回空）。"""
    if not text:
        return []
    head = text[:1024].lower()
    if "<!doctype" in head or "<!entity" in head:  # XXE/实体爆炸加固：直接拒绝
        return []
    try:
        root = ET.fromstring(text)
    except ET.ParseError:
        return []
    out = []
    for pm in root.iter():
        if pm.tag.rsplit("}", 1)[-1] != "ProbeMatch":
            continue
        xaddrs = scopes = ""
        for child in pm.iter():
            t = child.tag.rsplit("}", 1)[-1]
            if t == "XAddrs" and child.text:
                xaddrs = child.text.strip()
            elif t == "Scopes" and child.text:
                scopes = child.text.strip()
        name = ""
        sm = re.search(r"onvif://www\.onvif\.org/name/([^/\s]+)", scopes)
        if sm:
            name = sm.group(1)
        for url in xaddrs.split():
            try:
                host = (urlparse(url).hostname or "").strip("[]")
            except ValueError:
                continue
            if not host:
                continue
            out.append({"ip": host, "xaddr": url, "name": name, "scopes": scopes})
    return out


def ws_discover(timeout: float = 3.0) -> list[dict]:
    """组播一次 ONVIF Probe 并收集应答（按 ip 去重、只留局域网地址）。

    任何异常（无组播路由/防火墙/权限）都返回空列表，绝不抛——API 层负责给可读提示。
    """
    msg = _PROBE_TMPL.format(mid=uuid.uuid4()).encode("utf-8")
    found: dict[str, dict] = {}
    sock = None
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.settimeout(timeout)
        sock.sendto(msg, (WS_MULTICAST_ADDR, WS_PORT))
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                data, _addr = sock.recvfrom(65535)
            except socket.timeout:
                break
            except OSError:
                break
            for dev in _parse_probe_response(data.decode("utf-8", "replace")):
                if is_lan(dev["ip"]):
                    found.setdefault(dev["ip"], dev)
    except OSError:
        pass  # 套接字不可用（无组播路由等）：返回空
    finally:
        if sock is not None:
            try:
                sock.close()
            except Exception:  # noqa: BLE001  关闭失败不影响结果
                pass
    return list(found.values())


def get_devices(timeout: float = 3.0, refresh: bool = False) -> list[dict]:
    """带 30s 缓存的发现入口（避免页面刷新反复组播）。refresh=1 强制重发。"""
    with _cache_lock:
        if (not refresh and _cache["devices"]
                and time.time() - _cache["ts"] < CACHE_TTL_S):
            return list(_cache["devices"])
    devices = ws_discover(timeout)
    with _cache_lock:
        _cache.update({"ts": time.time(), "devices": list(devices)})
    return devices


def _find_ffprobe() -> str | None:
    hit = shutil.which("ffprobe")
    if hit:
        return hit
    for base in (Path(__file__).resolve().parents[2],  # 源码仓库根
                 Path(__file__).resolve().parents[3]):  # 便携布局（app/server/app → 包根）
        cand = base / "tools" / "ffmpeg" / "ffprobe.exe"
        if cand.exists():
            return str(cand)
    return None


def _ffprobe_stream(url: str) -> dict | None:
    """ffprobe 探测流信息；失败/不可用返回 None。**任何错误信息不得包含 url**（可能带凭证）。"""
    exe = _find_ffprobe()
    if not exe:
        return None
    try:
        r = subprocess.run(
            [exe, "-v", "error", "-rtsp_transport", "tcp", "-show_entries",
             "stream=codec_name,width,height,avg_frame_rate", "-of", "json", url],
            capture_output=True, timeout=FFPROBE_TIMEOUT_S)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if r.returncode != 0:
        return None
    try:
        import json
        streams = json.loads(r.stdout.decode("utf-8", "replace")).get("streams") or []
    except ValueError:
        return None
    for s in streams:  # 取第一个视频流
        if s.get("codec_name") in ("h264", "hevc", "h265", "mjpeg"):
            try:
                fps = float(eval_fraction(s.get("avg_frame_rate", "")))
            except (ValueError, ZeroDivisionError):
                fps = None
            return {"codec": s.get("codec_name"), "width": s.get("width"),
                    "height": s.get("height"), "fps": round(fps, 1) if fps else None}
    return None


def eval_fraction(text: str) -> float:
    """ffprobe 的 '30000/1001' 分数字符串 → float（纯函数可测；空/非法返回 0）。"""
    text = (text or "").strip()
    if not text:
        return 0.0
    num, _, den = text.partition("/")
    if not num:
        return 0.0
    try:
        return float(num) / float(den or "1")
    except ValueError:
        return 0.0


def probe_host(host: str, port: int = 554, timeout: float = 1.0,
               username: str | None = None, password: str | None = None) -> dict:
    """TCP/ffprobe 只读探测。凭证仅用于本次 ffprobe URL，不出现在返回值/日志。"""
    out = {"host": host, "port": port, "ip": None, "reachable": False,
           "port_open": False, "ffprobe": None, "error": ""}
    try:
        ip = socket.gethostbyname(host)
    except OSError:
        out["error"] = "主机名无法解析"
        return out
    out["ip"] = ip
    if not is_lan(ip):
        out["error"] = "仅允许探测局域网地址（10/172.16/192.168/169.254）"
        return out
    try:
        with socket.create_connection((ip, port), timeout=timeout):
            out["reachable"] = True
            out["port_open"] = True
    except OSError as e:
        out["error"] = f"端口 {port} 连接失败：{e.strerror or '超时或被拒绝'}"
        return out
    url = f"rtsp://{ip}:{port}/"
    if username:  # 给了凭证才注入（构造后立刻只在本函数内使用）
        url = f"rtsp://{quote(username, safe='')}:{quote(password or '', safe='')}@{ip}:{port}/"
    out["ffprobe"] = _ffprobe_stream(url)  # None=ffprobe 不可用或需认证/路径不对（不算失败）
    return out


def lan_addresses() -> list[str]:
    """本机局域网 IPv4（SPlayer-Next 同款排序：192.168/10 段优先，172.x Hyper-V/WSL 殿后）。"""
    ips: set[str] = set()
    try:
        infos = socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET)
    except OSError:
        return []
    for info in infos:
        ip = str(info[4][0])
        if is_lan(ip):
            ips.add(ip)
    def rank(ip: str):
        a = ipaddress.ip_address(ip)
        if a in ipaddress.ip_network("192.168.0.0/16"):
            return 0
        if a in ipaddress.ip_network("10.0.0.0/8"):
            return 1
        if a in ipaddress.ip_network("169.254.0.0/16"):
            return 2
        return 3  # 172.16/12（Hyper-V/WSL 虚拟网卡常见）
    return sorted(ips, key=rank)
