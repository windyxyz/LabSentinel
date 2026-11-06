"""LabSentinel 服务端入口。

运行: uvicorn app.main:app --port 8000
"""
import asyncio
import datetime
import gzip
import logging
import re
import threading
import time
import uuid
from collections import deque
from contextlib import asynccontextmanager
from contextvars import ContextVar
from pathlib import Path

from fastapi import APIRouter, FastAPI, Request, WebSocket, WebSocketDisconnect
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import (
    HTTPException as StarletteHTTPException,  # StaticFiles 抛的是 starlette 版
)

from . import (
    alert_config,
    backups,
    escalation,
    genai,
    go2rtc,
    keepawake,
    mqtt_bridge,
    plugins,
    retention,
)
from .alerts import dispatcher
from .api import init, router, ws_events
from .config import settings
from .logging_setup import (
    setup_file_logging,  # 任务61：导入阶段挂文件日志（尽早捕获启动错误）
)
from .pipeline import CameraManager
from .store import Store

setup_file_logging()  # LS_LOG_FILE=0 关闭；失败静默（console 照常）

# 任务95：启动配置校验（默认只告警；LS_STRICT_CONFIG=1 时 FAIL 拒绝启动）
from .validate import fail_count as _vc_fail
from .validate import validate_settings

_v_results = validate_settings()
for _v in _v_results:
    _lg = logging.getLogger("app.validate")
    if _v["level"] == "FAIL":
        _lg.error("配置校验 FAIL: %s %s（hint: %s）", _v["key"], _v["message"], _v["hint"])
    elif _v["level"] == "WARN":
        _lg.warning("配置校验 WARN: %s %s", _v["key"], _v["message"])
if settings.strict_config and _vc_fail(_v_results) > 0:
    _fails = [v for v in _v_results if v["level"] == "FAIL"]
    raise RuntimeError(
        f"配置校验 FAIL {_vc_fail(_v_results)} 项（LS_STRICT_CONFIG=1 拒绝启动）: "
        + "; ".join(f"{v['key']}: {v['message']}" for v in _fails))

# 请求 ID（任务63）：中间件写入，日志过滤器读取（见 logging_setup 的格式 [req=...]）
request_id_var: ContextVar[str] = ContextVar("request_id", default="-")


def _client_request_id(request: Request) -> str:
    rid = request.headers.get("X-Request-ID", "")
    if 1 <= len(rid) <= 128:
        return rid
    return uuid.uuid4().hex


def _rid(request: Request) -> str:
    return getattr(request.state, "request_id", request_id_var.get("-"))


def _face_compliance_notice():
    """人脸功能开启时必须打印合规提示（功能默认关闭；红线见 app/face.py）。"""
    print("=" * 64)
    print("[合规提示] 人脸比对功能已开启（LS_FACE_ENABLED=true）：")
    print("  1. 须已单独取得被识别人员同意并留痕（face_consents 表）")
    print("  2. 采集现场须设置显著的人脸识别提示标识")
    print("  3. 处理超过 10 万人人脸信息须依规备案")
    print("  4. 一键清除人员库与同意记录：DELETE /api/face/persons/all")
    print("=" * 64)


@asynccontextmanager
async def lifespan(app: FastAPI):
    dispatcher.hub.bind_loop(asyncio.get_running_loop())
    dispatcher.reset()  # 108：同一进程多次 lifespan（测试）时重新启用告警池
    store = Store()
    alert_config.load(store)  # 任务140：网页端告警配置（DB 覆盖 .env，保存即生效）
    dispatcher.bind_store(store)  # 任务13：投递记录/死信需要写库
    manager = CameraManager(store)
    init(store, manager)
    from . import engine
    engine.preload_async()  # B-28：后台预热推理引擎；失败不阻断启动，见 /api/system components
    if settings.face_enabled:
        _face_compliance_notice()
    manager.start_all()  # 启动已登记摄像头（文件源用于开发演示）
    manager.start_supervisor()  # 任务114：worker 退出自动按退避重启（24h 挂机保障）
    keepawake.start(settings.keep_awake, settings.keep_awake_display)  # 任务123：防休眠（默认关）
    genai.start_worker()  # 任务124：VLM 复核线程（provider=none 不启动）
    mqtt_bridge.start()  # 任务125：MQTT 事件总线（默认关）
    plugins.manager = plugins.PluginManager(Path(settings.data_dir) / "plugins")
    plugins.manager.autostart_enabled()  # 任务126：只拉起 enable+autostart 的插件
    go2rtc.sync_streams(store)  # 实时画面：按当前摄像头生成 go2rtc 配置并热重载
    retention.start_scheduler(store)  # 任务14：数据保留后台清理
    escalation.start_scheduler(store)  # 任务82：未处置事件升级提醒
    backups.start_scheduler()  # 任务92：定时自动备份
    try:
        yield
    finally:
        dispatcher.shutdown(wait=False, timeout=3)  # 108：先停告警派发，退出不被重试拖住
        manager.stop_all()
        retention.stop_scheduler()
        escalation.stop_scheduler()
        backups.stop_scheduler()
        keepawake.stop()  # 任务123：退出恢复电源语义（与断言同线程解除）
        genai.stop_worker()
        mqtt_bridge.stop()
        if plugins.manager is not None:
            plugins.manager.stop_all()


from .version import read_version as _read_version

app = FastAPI(title="LabSentinel", version=_read_version(), lifespan=lifespan)
app.include_router(router, prefix="/api")  # 兼容别名（弃用期两个版本）
app.include_router(router, prefix="/api/v1", include_in_schema=False)  # T-621 正式前缀
# T-621（§9.4）：API 版本化——/api/v1 为正式契约前缀（/api 保留为别名，弃用期两个版本）
_v1 = APIRouter(prefix="/api/v1")
from . import constants as _constants  # noqa: E402


@_v1.get("/meta")
def api_meta():
    """类型/级别/状态文案单一来源（前端/移动端运行时拉取，消除手工同步）。"""
    from .export import STATUS_TEXT as _st
    return {"types": _constants.TYPE_TEXT, "statuses": _st,
            "levels": {"1": "提示", "2": "警告", "3": "严重"},
            "version": __import__("app.version", fromlist=["read_version"]).read_version()}


app.include_router(_v1)

# ---- T-105（S-03/P1）：Host 白名单（防 DNS Rebinding）----
from starlette.middleware.trustedhost import TrustedHostMiddleware  # noqa: E402

_allowed_hosts = [h.strip() for h in settings.trusted_hosts.split(",") if h.strip()] + ["testserver"]
app.add_middleware(TrustedHostMiddleware, allowed_hosts=_allowed_hosts)

_UNSAFE_METHODS = {"POST", "PUT", "PATCH", "DELETE"}


def _same_origin(request: Request) -> bool:
    """T-105：写请求同源校验（Sec-Fetch-Site 优先，其次 Origin；无 Origin 视为非浏览器客户端放行）。"""
    sfs = request.headers.get("sec-fetch-site")
    if sfs is not None:
        return sfs in ("same-origin", "none")  # none = 地址栏/书签/非浏览器
    origin = request.headers.get("origin")
    if origin is None:
        return True
    host = request.headers.get("host", "")
    if origin.split("://", 1)[-1] == host:
        return True
    allowed = [o.strip() for o in settings.allowed_origins.split(",") if o.strip()]
    return origin in allowed


def _security_headers(response: Response) -> None:
    """任务89 安全响应头（T-113/S-18：抽成函数，500 分支也调用）。"""
    if not settings.security_headers:
        return
    h = response.headers
    h.setdefault("X-Content-Type-Options", "nosniff")
    h.setdefault("X-Frame-Options", settings.frame_options)
    h.setdefault("Referrer-Policy", "no-referrer")
    h.setdefault("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
    h.setdefault("Cross-Origin-Resource-Policy", "same-site")
    if settings.csp:
        name = "Content-Security-Policy-Report-Only" if settings.csp_report_only \
            else "Content-Security-Policy"
        h.setdefault(name, settings.csp)

# ---- 写接口限流（任务90）：滑动窗口，内存实现，单进程 ----
_rate_lock = threading.Lock()
_rate_windows: dict[str, deque] = {}  # client_ip → [timestamps]


def _client_ip(request: Request) -> str:
    """任务90：trust_proxy 开启时取 X-Forwarded-For 第一跳；否则直连地址。"""
    if settings.trust_proxy:
        xff = request.headers.get("X-Forwarded-For", "")
        if xff:
            return xff.split(",")[0].strip()
    return request.client.host if request.client else "unknown"


@app.middleware("http")
async def request_id_middleware(request: Request, call_next):
    """任务63：响应头始终带 X-Request-ID（客户端合法传入则回显，否则生成 uuid4）。
    任务87：写请求（/api/* 非 GET）完成后落审计行（不含 body/headers/query）。
    任务90：/api/* 写接口滑动窗口限流（GET/WS/健康探针不受影响）。"""
    rid = _client_request_id(request)
    request.state.request_id = rid
    request_id_var.set(rid)
    # T-105：跨站写请求拒绝（防任意网页 fetch POST /api/system/shutdown 等简单请求 CSRF）
    if (request.method in _UNSAFE_METHODS and request.url.path.startswith("/api/")
            and not _same_origin(request)):
        return JSONResponse({"detail": "跨站请求被拒绝", "request_id": rid,
                             "code": "csrf_blocked"},
                            status_code=403,
                            headers={"X-Request-ID": rid, **({"Vary": "Origin"})})
    # 任务90：写接口限流（在 request_id 之后、业务之前；429 直接返回）
    if (request.method in ("POST", "PUT", "PATCH", "DELETE")
            and request.url.path.startswith("/api/")
            and settings.write_rate_limit > 0):
        ip = _client_ip(request)
        now = time.monotonic()
        window = settings.write_rate_window_s
        with _rate_lock:
            dq = _rate_windows.setdefault(ip, deque())
            while dq and dq[0] < now - window:
                dq.popleft()
            if len(dq) >= settings.write_rate_limit:
                retry_after = max(1, window - int(now - dq[0]))
                logging.getLogger("app.errors").warning(
                    "[rate-limit] %s %s %s 超限（%d/%d per %ds）", ip,
                    request.method, request.url.path, len(dq), settings.write_rate_limit, window)
                return JSONResponse(
                    {"detail": "写操作过于频繁，请稍后重试", "request_id": rid,
                     "code": "rate_limited"},
                    status_code=429,
                    headers={"X-Request-ID": rid, "Retry-After": str(retry_after)})
            dq.append(now)
            if len(_rate_windows) > 1000:
                cutoff = now - window
                for k in [k for k, v in _rate_windows.items() if not v or v[-1] < cutoff]:
                    del _rate_windows[k]
    try:
        response = await call_next(request)
    except Exception:  # noqa: BLE001  某些中间件层异常不走 exception_handler，也兜住
        logging.getLogger("app.errors").exception(
            "[req=%s] %s %s 未捕获异常（中间件层）", rid, request.method, request.url.path)
        # 75 修复：错误体带 request_id，响应头也要带（原实现漏了头，前端拿不到关联 ID）
        resp500 = JSONResponse({"detail": "服务器内部错误", "request_id": rid, "code": "internal_error"},
                               status_code=500, headers={"X-Request-ID": rid})
        _security_headers(resp500)  # S-18：500 分支不再缺安全头
        return resp500
    response.headers["X-Request-ID"] = rid
    # 任务87：审计（/api/* 写操作；写失败只 WARNING 不影响响应；不含 body/headers/query）
    if (request.method in _UNSAFE_METHODS
            and request.url.path.startswith("/api/")):
        try:
            # T-113：actor 由 require_admin 判定结果写入 state；401 记 rejected
            actor = getattr(request.state, "actor", None)
            if not actor:
                actor = "rejected" if response.status_code == 401 else \
                        ("admin" if request.headers.get("X-Admin-Token") else "anonymous")
            _audit_write(datetime.datetime.now().isoformat(timespec="seconds"), rid, actor,
                         f"{request.method} {request.url.path}", response.status_code)
        except Exception as e:  # noqa: BLE001
            logging.getLogger("app.errors").warning("[audit] 写审计失败: %s", e)
    _security_headers(response)
    return response


def _audit_write(ts: str, rid: str, actor: str, action: str, status_code: int) -> None:
    from .store import connect
    m = re.search(r"/(\d+)(?:/|$)", action)
    target = None
    if m:
        kind = ("camera" if "/cameras/" in action else
                "event" if "/events/" in action else
                "zone" if "/zones/" in action else
                "rule" if "/rules/" in action else
                "silence" if "/alert-silences/" in action else "id")
        target = f"{kind}:{m.group(1)}"
    c = connect(_audit_db_path())
    try:
        c.execute("INSERT INTO audit_log(ts,request_id,actor,action,target,status_code) "
                  "VALUES(?,?,?,?,?,?)", (ts, rid, actor, action, target, status_code))
        c.commit()
    finally:
        c.close()


def _audit_db_path() -> str:
    from .config import settings
    return str(settings.db_path)


@app.exception_handler(StarletteHTTPException)
async def http_exc_handler(request: Request, exc: StarletteHTTPException):
    body = {"detail": exc.detail,
            "request_id": _rid(request),
            "code": f"http_{exc.status_code}"}
    return JSONResponse(body, status_code=exc.status_code, headers=getattr(exc, "headers", None))


@app.exception_handler(RequestValidationError)
async def validation_exc_handler(request: Request, exc: RequestValidationError):
    first = exc.errors()[0] if exc.errors() else {}
    loc = ".".join(str(x) for x in first.get("loc", []))
    summary = f"{loc}: {first.get('msg', '参数校验失败')}" if loc else "参数校验失败"
    return JSONResponse({"detail": summary, "request_id": _rid(request), "code": "validation_error"},
                        status_code=422)


@app.exception_handler(Exception)
async def unhandled_exc_handler(request: Request, exc: Exception):
    rid = _rid(request)
    # 用 app.* logger（向 root 传播 → server.log 能按 [req=...] 检索）；uvicorn.error 不向 root 传播
    logging.getLogger("app.errors").exception(
        "[req=%s] %s %s 未捕获异常", rid, request.method, request.url.path)  # 堆栈只进日志
    return JSONResponse({"detail": "服务器内部错误", "request_id": rid, "code": "internal_error"},
                        status_code=500)


@app.websocket("/ws/events")
async def events_ws(ws: WebSocket):
    await ws_events(ws)


@app.websocket("/ws/webrtc/{cid}")
async def webrtc_ws(ws: WebSocket, cid: int):
    """T-102：WebRTC 信令鉴权代理——浏览器只连 8000，go2rtc :1984 仅回环可达。"""
    from . import webrtc_proxy
    from .api import _ws_auth_check
    if not _ws_auth_check(ws):
        await ws.close(code=4401)
        return
    await ws.accept()
    try:
        await webrtc_proxy.proxy(ws, cid)
    except (WebSocketDisconnect, OSError, ConnectionError):
        pass
    except Exception as e:  # noqa: BLE001 上游不可用时不拖垮服务
        from .logging_setup import get_logger
        get_logger("app.webrtc").warning("WebRTC 代理异常: %s: %s", type(e).__name__, e)


class SPAStaticFiles(StaticFiles):
    """静态托管 + SPA 回退（任务43）+ 缓存策略（任务72）：
    - 404 且路径末段无扩展名 → 回 index.html，让 /events 等前端路由刷新不 404；
    - Cache-Control：assets/*（Vite 内容 hash）→ immutable 一年；index.html 与 SPA 回退 → no-cache
      （保证升级立即生效）；其余静态 → 1h；304 响应同样携带；/api、/ws 不经过此处。"""

    async def get_response(self, path: str, scope):
        # Windows 上 StaticFiles 传入的 path 用反斜杠（如 api\x），统一成 / 再判断
        norm = path.replace("\\", "/").lstrip("/")
        try:
            resp = await super().get_response(norm, scope)
        except StarletteHTTPException as e:
            if (e.status_code == 404 and "." not in Path(norm).name
                    and not norm.startswith(("api/", "ws/"))):
                # 未知 API/WS 路径保持 404，不能被 SPA 回退成 HTML（客户端会误解析）
                resp = await super().get_response("index.html", scope)
                resp.headers["Cache-Control"] = "no-cache"
                return resp
            raise
        first = norm.split("/")[0] if norm else ""
        if first == "assets":
            resp.headers["Cache-Control"] = "public, max-age=31536000, immutable"
        elif norm.strip("/.") in ("", "index.html"):  # "." 是 StaticFiles 对 "/" 的内部表示
            resp.headers["Cache-Control"] = "no-cache"
        else:
            resp.headers["Cache-Control"] = "public, max-age=3600"
        return resp


# ---- 选择性 gzip（任务72）：只压文本类；Range/已压缩/小体积极跳过；视频/zip/图片不压 ----
GZIP_MIN_BYTES = 1024
GZIP_TYPE_PREFIXES = ("text/", "image/svg+xml", "application/javascript", "application/json")


@app.middleware("http")
async def selective_gzip_middleware(request: Request, call_next):
    resp = await call_next(request)
    if "gzip" not in request.headers.get("accept-encoding", "").lower():
        return resp
    if request.headers.get("range"):
        return resp  # Range（视频/证据包断点续传）优先，gzip 让路
    ct = resp.headers.get("content-type", "").split(";")[0].strip().lower()
    if not ct.startswith(GZIP_TYPE_PREFIXES) or resp.headers.get("content-encoding"):
        return resp
    if resp.headers.get("content-disposition", "").lower().startswith("attachment"):
        return resp  # 2026-10-30 审阅：CSV 等下载是流式生成，整体缓冲进内存会击穿流式设计
    # 任务77：大响应/未知大小（流式）保护——只压 Content-Length 已知且 ≤ 阈值的响应
    cl = resp.headers.get("content-length")
    if cl is None:
        return resp  # chunked/streaming：避免缓冲未知大小流
    try:
        if int(cl) > settings.gzip_max_bytes:
            return resp
    except ValueError:
        return resp
    if resp.status_code == 304:
        resp.headers.setdefault("Vary", "Accept-Encoding")
        return resp
    body = b""
    async for chunk in resp.body_iterator:
        body += chunk
    if len(body) < GZIP_MIN_BYTES:
        return Response(content=body, status_code=resp.status_code,
                        headers=dict(resp.headers), media_type=None)
    gz = gzip.compress(body, compresslevel=6)
    headers = {k: v for k, v in resp.headers.items() if k.lower() != "content-length"}
    headers["Content-Encoding"] = "gzip"
    headers["Vary"] = "Accept-Encoding"
    return Response(content=gz, status_code=resp.status_code, headers=headers, media_type=None)


def mount_web(app: FastAPI = None) -> None:
    """单端口模式：托管 web/dist（存在才挂，GET / 直接出控制面板，无需 Vite）。
    可重入：先移除旧挂载/占位路由（测试 monkeypatch settings 后可重挂）。"""
    app = app or globals()["app"]
    app.router.routes[:] = [r for r in app.router.routes if getattr(r, "name", "") != "web_dist"]
    d = settings.web_dist
    if (d / "index.html").is_file():
        app.mount("/", SPAStaticFiles(directory=str(d), html=True), name="web_dist")
    else:
        @app.get("/", name="web_dist", include_in_schema=False)
        async def _dist_missing():
            return {"message": "web dist not built; run npm run build（或设 LS_WEB_DIST 指向已构建目录）"}


mount_web(app)
