"""LabSentinel 服务端入口。

运行: uvicorn app.main:app --port 8000
"""
import asyncio
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import FastAPI, WebSocket
from fastapi.staticfiles import StaticFiles
from starlette.exceptions import HTTPException as StarletteHTTPException  # StaticFiles 抛的是 starlette 版

from .alerts import dispatcher
from .api import init, router, ws_events
from . import go2rtc
from . import retention
from .config import settings
from .pipeline import CameraManager
from .store import Store


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


@app.websocket("/ws/events")
async def events_ws(ws: WebSocket):
    await ws_events(ws)


class SPAStaticFiles(StaticFiles):
    """静态托管 + SPA 回退（任务43）：404 且路径末段无扩展名 → 回 index.html，
    让 /events 等前端路由刷新不 404；带扩展名的缺失资源仍 404，避免把坏 js 伪装成 html。"""

    async def get_response(self, path: str, scope):
        # Windows 上 StaticFiles 传入的 path 用反斜杠（如 api\x），统一成 / 再判断
        norm = path.replace("\\", "/").lstrip("/")
        try:
            return await super().get_response(norm, scope)
        except StarletteHTTPException as e:
            if (e.status_code == 404 and "." not in Path(norm).name
                    and not norm.startswith(("api/", "ws/"))):
                # 未知 API/WS 路径保持 404，不能被 SPA 回退成 HTML（客户端会误解析）
                return await super().get_response("index.html", scope)
            raise


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
