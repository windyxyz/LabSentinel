"""LabSentinel 服务端入口。

运行: uvicorn app.main:app --port 8000
"""
import asyncio
from contextlib import asynccontextmanager

from fastapi import FastAPI, WebSocket

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
