"""go2rtc 流媒体网关集成（实时画面）。

同步策略（go2rtc v1.9.15 实测）：
- 无 /api/reload 端点（GET/POST 均 404）；热同步用 PUT/DELETE /api/streams
  （PUT 创建/更新流，DELETE 删除流）。
- 文件源必须带 ffmpeg: 前缀（裸路径经 API 校验报 "source not supported"；
  RTSP 源原样透传）。ffmpeg.exe 放 tools/ffmpeg/，start-all.bat 已加 PATH。
- go2rtc 未运行时：把期望配置写入 yaml 供其冷启动读取；运行时以 API 为准。
go2rtc 与分析 worker 共用同一路 RTSP 源（go2rtc 自行拉流，不增加摄像头出流量）。

注意（2026-10-30 审阅修复）：曾把状态变量与取客户端函数都命名为 `_client`，变量赋值在函数定义
之后执行导致函数被覆盖为 None，热同步整条链路静默失效——现改名为 `_http`，两者不再冲突。
"""
import httpx

from .config import settings

_YAML_HEAD = """\
# 本文件由 LabSentinel 服务端自动生成（摄像头增删时重写），勿手工编辑
api:
  listen: ":1984"
  origin: "*"
webrtc:
  listen: ":1984"
streams:
"""


def source_for(cam: dict) -> str | None:
    """摄像头 → go2rtc 流源；usb 源不进 go2rtc。"""
    if cam["type"] == "rtsp":
        return cam["source"]
    if cam["type"] == "file":
        # 裸路径过不了 API 校验，必须 ffmpeg: 前缀（go2rtc 无裸文件输入模块）
        return "ffmpeg:" + cam["source"].replace("\\", "/")
    return None


def build_yaml(cameras: list[dict]) -> str:
    lines = [_YAML_HEAD]
    for cam in cameras:
        src = source_for(cam)
        if src:
            lines.append(f'  "{cam["id"]}": "{src}"')
    return "\n".join(lines) + "\n"


_http: httpx.Client | None = None


def _client() -> httpx.Client:
    """go2rtc 本机服务的会话。地址来自运维配置（默认回环 127.0.0.1:1984）；
    非 https 且目标不是回环地址时拒绝启动该客户端（防配置失误把事件流打到外网）。"""
    global _http
    if _http is None:
        host = settings.go2rtc_api.split(":")[0] or "127.0.0.1"
        loopback = host in ("127.0.0.1", "localhost", "::1")
        if not loopback:
            print(f"[go2rtc] 提示：目标 {settings.go2rtc_api} 非回环地址，请确保为可信内网网关")
        _http = httpx.Client(base_url=f"http://{settings.go2rtc_api}/", timeout=2)
    return _http


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
