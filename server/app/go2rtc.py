"""go2rtc 流媒体网关集成（实时画面）。

同步策略（go2rtc v1.9.15 实测）：
- 无 /api/reload 端点；热同步用 PUT/DELETE /api/streams。
- 文件源必须带 ffmpeg: 前缀；RTSP 源原样透传（已由 sources.validate_source 校验）。
- go2rtc 未运行时：把期望配置写入 yaml 供其冷启动读取；运行时以 API 为准。

安全（T-101 / S-01，P0）：
- api.listen 只绑 127.0.0.1:1984、删除 origin:"*"；webrtc.listen 独立 :8555；
  rtsp 再分发关闭（listen: ""）。
- api.username/password 首次随机生成写入 <data_dir>/go2rtc.secret（POSIX 0600）；
  所有本机 API 调用走 httpx.Client(auth=...)。
- 浏览器不再直连 1984：WebRTC 信令经 FastAPI /ws/webrtc/{cid} 鉴权代理（T-102）。

注意（2026-10-30 审阅修复）：曾把状态变量与取客户端函数都命名为 `_client`，变量赋值在函数定义
之后执行导致函数被覆盖为 None，热同步整条链路静默失效——现改名为 `_http`，两者不再冲突。
"""
import json
import os
import secrets
from pathlib import Path

import httpx

from .config import settings
from .sources import yaml_stream_entry

#: T-101：控制面只绑回环、无通配 origin；媒体面独立端口；RTSP 再分发关闭
_API_LISTEN = "127.0.0.1:1984"
_WEBRTC_LISTEN = ":8555"


def _secret_path() -> Path:
    return Path(settings.data_dir) / "go2rtc.secret"


def _secret() -> tuple[str, str]:
    """读或生成 go2rtc API 凭据（首次 secrets.token_urlsafe）。"""
    p = _secret_path()
    try:
        data = json.loads(p.read_text("ascii"))
        if data.get("username") and data.get("password"):
            return data["username"], data["password"]
    except (OSError, ValueError):
        pass
    user, pwd = "ls-" + secrets.token_urlsafe(12), secrets.token_urlsafe(24)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({"username": user, "password": pwd}), encoding="ascii")
    if os.name == "posix":
        try:
            p.chmod(0o600)
        except OSError:
            pass
    return user, pwd


def yaml_head() -> str:
    """生成带鉴权的 YAML 头（凭据只在本地文件/进程内，绝不外发）。"""
    user, pwd = _secret()
    lines = [
        "# 本文件由 LabSentinel 服务端自动生成（摄像头增删时重写），勿手工编辑",
        "api:",
        f'  listen: "{_API_LISTEN}"',
        f"  username: {json.dumps(user)}",
        f"  password: {json.dumps(pwd)}",
        "# origin: 删除（禁止任意网页跨域读控制面）",
        "webrtc:",
        f'  listen: "{_WEBRTC_LISTEN}"  # 媒体面（UDP/TCP），仅承载 SRTP',
        "rtsp:",
        '  listen: ""  # 关闭 RTSP 再分发（默认 :8554 会把所有流对外发布）',
        "log:",
        "  level: warn",
        "streams:",
    ]
    return "\n".join(lines)


def source_for(cam: dict) -> str | None:
    """摄像头 → go2rtc 流源；usb/folder 源不进 go2rtc。"""
    if cam["type"] == "rtsp":
        return cam["source"]
    if cam["type"] == "file":
        # 裸路径过不了 API 校验，必须 ffmpeg: 前缀（go2rtc 无裸文件输入模块）
        return "ffmpeg:" + cam["source"].replace("\\", "/")
    return None


def build_yaml(cameras: list[dict]) -> str:
    lines = [yaml_head()]
    for cam in cameras:
        src = source_for(cam)
        if src:
            lines.append(yaml_stream_entry(cam["id"], src))
    return "\n".join(lines) + "\n"


_http: httpx.Client | None = None


def _client() -> httpx.Client:
    """go2rtc 本机服务的会话：固定回环地址 + Basic 鉴权（T-101）。"""
    global _http
    if _http is None:
        user, pwd = _secret()
        _http = httpx.Client(base_url=f"http://{_API_LISTEN}/", auth=(user, pwd), timeout=2)
    return _http


def health() -> dict:
    """T-202/F-09：go2rtc 健康（带凭据 GET /api/streams）；不抛异常，供 components。"""
    try:
        r = _client().get("api/streams")
        return {"ok": r.status_code == 200, "status_code": r.status_code}
    except httpx.HTTPError as e:
        return {"ok": False, "error": type(e).__name__}


def sync_streams(store) -> None:
    """按当前摄像头列表同步 go2rtc：优先 API 热同步，失败则写 yaml 冷启动。"""
    desired = {}
    for cam in store.cameras():
        src = source_for(cam)
        if src:
            desired[str(cam["id"])] = src

    try:  # 热同步（go2rtc 在运行）
        current = _client().get("api/streams").json()
        changed = 0
        for name, src in desired.items():
            if current.get(name, {}).get("producers") != [{"url": src}]:
                r = _client().put("api/streams", params={"name": name, "src": src}, timeout=10)
                if r.status_code == 200:
                    changed += 1
                else:
                    print(f"[go2rtc] PUT 流 {name} 失败: {r.status_code} {r.text[:80]}")
        for name in current:
            if name not in desired:
                _client().delete("api/streams", params={"src": name}, timeout=5)
                changed += 1
        if changed:
            print(f"[go2rtc] 已热同步 {changed} 路流（共 {len(desired)} 路）")
        return
    except httpx.HTTPError:
        pass  # 连接失败 = go2rtc 未运行，走冷启动
    except Exception as e:  # noqa: BLE001  非连接类异常必须留痕（2026-10-30 审阅：曾静默吞掉）
        print(f"[go2rtc] 热同步异常（转冷启动）: {type(e).__name__}: {e}")

    try:  # 冷启动路径：写 yaml，go2rtc 下次启动时读取
        path = settings.go2rtc_yaml
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(build_yaml(list(store.cameras())), encoding="utf-8")
        print(f"[go2rtc] 服务未运行，配置已写入 {path}（共 {len(desired)} 路，启动 go2rtc 后生效）")
    except OSError as e:
        print(f"[go2rtc] 配置写入失败: {e}")
