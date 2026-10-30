"""LabSentinel 服务端入口。

运行: uvicorn app.main:app --port 8000
"""
import asyncio
import gzip
import logging
import uuid
from contextlib import asynccontextmanager
from contextvars import ContextVar
from pathlib import Path

from fastapi import FastAPI, Request, WebSocket
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import HTTPException as StarletteHTTPException  # StaticFiles 抛的是 starlette 版

from .alerts import dispatcher
from .api import init, router, ws_events
from . import go2rtc
from . import retention
from .config import settings
from .logging_setup import setup_file_logging  # 任务61：导入阶段挂文件日志（尽早捕获启动错误）
from .pipeline import CameraManager
from .store import Store

setup_file_logging()  # LS_LOG_FILE=0 关闭；失败静默（console 照常）

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
    store = Store()
    dispatcher.bind_store(store)  # 任务13：投递记录/死信需要写库
    manager = CameraManager(store)
    init(store, manager)
    if settings.face_enabled:
        _face_compliance_notice()
    manager.start_all()  # 启动已登记摄像头（文件源用于开发演示）
    go2rtc.sync_streams(store)  # 实时画面：按当前摄像头生成 go2rtc 配置并热重载
    retention.start_scheduler(store)  # 任务14：数据保留后台清理
    try:
        yield
    finally:
        manager.stop_all()
        retention.stop_scheduler()


app = FastAPI(title="LabSentinel", version="0.1.0-m1", lifespan=lifespan)
app.include_router(router)


@app.middleware("http")
async def request_id_middleware(request: Request, call_next):
    """任务63：响应头始终带 X-Request-ID（客户端合法传入则回显，否则生成 uuid4）。"""
    rid = _client_request_id(request)
    request.state.request_id = rid
    request_id_var.set(rid)
    try:
        response = await call_next(request)
    except Exception:  # noqa: BLE001  某些中间件层异常不走 exception_handler，也兜住
        logging.getLogger("app.errors").exception(
            "[req=%s] %s %s 未捕获异常（中间件层）", rid, request.method, request.url.path)
        # 75 修复：错误体带 request_id，响应头也要带（原实现漏了头，前端拿不到关联 ID）
        return JSONResponse({"detail": "服务器内部错误", "request_id": rid, "code": "internal_error"},
                            status_code=500, headers={"X-Request-ID": rid})
    response.headers["X-Request-ID"] = rid
    return response


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
