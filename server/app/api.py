"""REST API 路由。"""
import csv
import io
import os
import shutil
import sqlite3
import time
from datetime import datetime, timedelta
from pathlib import Path

import cv2
from fastapi import APIRouter, Depends, File, Form, HTTPException, Request, UploadFile, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, JSONResponse, Response, StreamingResponse

from . import go2rtc
from .alerts import dispatcher, render_alert
from .config import settings
from .engine import get_engine
from .face import FaceClient
from .models import CameraCreate, RuleCreate, ZoneCreate
from .pipeline import CameraManager
from .stats import daily as stats_daily_query
from .stats import hourly as stats_hourly_query
from .stats import summary as stats_summary_query
from .stats import weekly as stats_weekly_query
from .store import EVENT_STATUSES, Store

router = APIRouter(prefix="/api")

store: Store | None = None
manager: CameraManager | None = None
face_client = FaceClient()  # 依据 settings 配置；未配置时 configured=False

# ---- 鉴权（任务16）：LS_ADMIN_TOKEN 为空 = 不启用；写操作需 X-Admin-Token ----
WS_AUTH_CODE = 4401


def _token_ok(provided: str | None) -> bool:
    if not settings.admin_token:
        return True  # 未启用鉴权
    if not provided:
        return False
    import hmac
    return hmac.compare_digest(provided, settings.admin_token)


def require_admin(request: Request):
    """写操作守卫：Depends(require_admin) 挂在 POST/PUT/DELETE 路由上。"""
    if not _token_ok(request.headers.get("X-Admin-Token")):
        raise HTTPException(401, "需要管理令牌（X-Admin-Token）")


def require_read(request: Request):
    """读操作守卫：仅 LS_AUTH_READ_PROTECTED=true 时生效。

    105 回归补丁 P0-2：`<img>/<video>/<a download>` 裸 URL 不带 X-Admin-Token 头，读保护开启后
    截图/视频/导出全 401——接受 `?token=` 查询参数作为回退（与 header 同用 hmac.compare_digest）。
    URL token 会进浏览器历史/代理日志，文档建议配合独立令牌与 Referrer-Policy（任务 89）。
    """
    if not settings.auth_read_protected:
        return
    provided = request.headers.get("X-Admin-Token") or request.query_params.get("token")
    if not provided or not _token_ok(provided):
        raise HTTPException(401, "需要管理令牌（X-Admin-Token 头或 ?token= 查询参数）")


def init(s: Store, m: CameraManager):
    global store, manager
    store, manager = s, m


def _event_row(row: dict) -> dict:
    row = dict(row)
    row["has_snapshot"] = bool(row.get("snapshot"))
    row["has_video"] = bool(row.get("video"))
    row["status"] = row.get("status") or "new"  # 旧库迁移前列缺失时兜底
    return row


# ---- health（任务63）：轻量探针，永不触发推理 ----
@router.get("/health/live")
def health_live():
    """存活探针：进程活着即 200；不查 DB、不读模型。"""
    return {"status": "ok"}


@router.get("/health/ready")
def health_ready():
    """就绪探针：逐项检查，全过 200，否则 503。不加载 ONNX 会话（避免 DML 并发）。"""
    import shutil
    checks = {}

    try:
        con = sqlite3.connect(f"file:{store.path}?mode=ro", uri=True)
        con.execute(f"PRAGMA busy_timeout={int(settings.sqlite_busy_timeout_ms)}")
        try:
            con.execute("SELECT 1").fetchone()
        finally:
            con.close()
        checks["db"] = {"ok": True, "detail": "select 1 ok"}
    except Exception as e:  # noqa: BLE001
        checks["db"] = {"ok": False, "detail": str(e)[:120]}

    try:
        probe = settings.data_dir / ".ready_probe"
        probe.write_text("ok", encoding="ascii")
        probe.unlink(missing_ok=True)
        checks["data_dir"] = {"ok": True, "detail": "可写"}
    except OSError as e:
        checks["data_dir"] = {"ok": False, "detail": f"不可写: {e}"[:120]}

    model = settings.ppe_model or settings.model_fp32
    checks["model"] = {"ok": bool(model and Path(model).exists()),
                       "detail": str(model or "(未配置)")}

    try:
        free_gb = shutil.disk_usage(settings.data_dir).free / 1e9
        checks["disk"] = {"ok": free_gb >= settings.ready_min_free_gb,
                          "free_gb": round(free_gb, 1)}
    except OSError as e:
        checks["disk"] = {"ok": False, "free_gb": None, "detail": str(e)[:120]}

    ready = all(v.get("ok") for v in checks.values())
    return JSONResponse({"status": "ready" if ready else "degraded", "checks": checks},
                        status_code=200 if ready else 503)


# ---- system ----
def _system_version() -> str:
    return "0.1.0-m1"


@router.get("/system")
def system():
    today = store.today_event_counts()
    unhandled = sum(n for lv, n in today.items()) - store.count_events_today_handled()
    return {
        "version": _system_version(),
        "schema_version": store.schema_version(),  # 任务69
        "backend": get_engine().backend,
        **(manager.status() if manager else {"cameras": []}),
        "today_counts": today,
        "unhandled_today": max(0, unhandled),  # 今日未处置（任务12）
        "alerts": dispatcher.store.delivery_counts() if dispatcher.store else {"pending": 0, "failed": 0, "ok": 0},
    }


# ---- cameras ----
# 2026-10-30 审阅：读保护口径统一——/api/system 与 /api/health/* 例外永不拦截（启动器健康检查用），
# 其余 GET（cameras/zones/rules/snapshot/webrtc/stats/maintenance/export）一律 require_read。
# 此前 export/evidence.zip、实时快照等含敏感数据的端点漏挂，读保护形同虚设。
@router.get("/cameras", dependencies=[Depends(require_read)])
def cameras():
    return store.cameras()


@router.get("/cameras/usb-devices", dependencies=[Depends(require_read)])
def usb_devices(refresh: int = 0):
    """枚举本机 DirectShow USB 摄像头（任务60）。

    index 为枚举顺序（通常与 CAP_DSHOW 一致，驱动/插拔变化后可能改变，设备名更稳定）；
    失败一律 200 + devices=[] + 中文 error（前端回退手输索引）。
    """
    from . import usb_devices as usb
    return usb.list_devices(refresh=bool(refresh))


@router.post("/cameras", dependencies=[Depends(require_admin)])
def add_camera(body: CameraCreate):
    if body.type not in ("rtsp", "file", "usb"):
        raise HTTPException(400, "type must be rtsp|file|usb")
    cid = store.add_camera(body.name, body.source, body.type)
    if body.autostart:
        manager.start_camera(store.camera(cid))
    go2rtc.sync_streams(store)  # 实时画面：同步流媒体网关配置
    return store.camera(cid)


@router.post("/cameras/{cid}/start", dependencies=[Depends(require_admin)])
def start_camera(cid: int):
    cam = store.camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    manager.start_camera(cam)
    return {"ok": True}


@router.delete("/cameras/{cid}", dependencies=[Depends(require_admin)])
def delete_camera(cid: int):
    manager.stop_camera(cid)
    store.delete_camera(cid)
    go2rtc.sync_streams(store)  # 实时画面：同步流媒体网关配置
    return {"ok": True}


@router.get("/cameras/{cid}/webrtc", dependencies=[Depends(require_read)])
def camera_webrtc(cid: int, request: Request):
    """返回该路流的 go2rtc WebRTC 信令地址（浏览器直连 go2rtc :1984）。"""
    if store.camera(cid) is None:
        raise HTTPException(404, "camera not found")
    host = request.url.hostname or "localhost"
    return {"camera_id": cid, "ws_url": f"ws://{host}:1984/api/ws?src={cid}"}


@router.get("/cameras/{cid}/status", dependencies=[Depends(require_read)])
def camera_status(cid: int):
    """单路运行指标（任务65）：帧龄/最近错误（已脱敏）/重连次数/累计帧数。"""
    cam = store.camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    w = manager.workers.get(cid) if manager else None
    out = {"id": cid, "name": cam["name"], "type": cam["type"], "status": cam["status"],
           "fps": None, "last_frame_ts": None, "frame_age_s": None, "last_error": None,
           "last_error_ts": None, "reconnect_count": 0, "opened_at": None, "frames_total": 0}
    if w is not None:
        lfw = w.last_frame_ts
        out.update({
            "fps": round(w.analysis_fps_now, 1),
            "last_frame_ts": lfw,
            "frame_age_s": round(time.time() - lfw, 1) if lfw else None,
            "last_error": w.last_error,
            "last_error_ts": w.last_error_ts,
            "reconnect_count": w.reconnect_count,
            "opened_at": w.opened_at,
            "frames_total": w.frames_total,
        })
    return out


@router.get("/cameras/{cid}/snapshot", dependencies=[Depends(require_read)])
def camera_snapshot(cid: int):
    frame = manager.latest_frame(cid)
    if frame is None:
        raise HTTPException(404, "no frame yet")
    import cv2
    ok, buf = cv2.imencode(".jpg", frame)
    if not ok:
        raise HTTPException(500, "encode failed")
    from fastapi.responses import Response
    return Response(content=buf.tobytes(), media_type="image/jpeg")


# ---- zones ----
@router.get("/zones", dependencies=[Depends(require_read)])
def zones(camera_id: int | None = None):
    return store.zones(camera_id)


@router.post("/zones", dependencies=[Depends(require_admin)])
def add_zone(body: ZoneCreate):
    if store.camera(body.camera_id) is None:
        raise HTTPException(404, "camera not found")
    # 2026-10-30 审阅：入库前校验——坏 polygon 原先会静默入库，worker 每帧 parse 抛错，
    # 摄像头显示 running 却零事件。parse_polygon 的 ValueError 也在此一并转 400。
    if len(body.polygon) < 3:
        raise HTTPException(400, "安全区至少需要 3 个顶点")
    if any(len(pt) != 2 for pt in body.polygon):
        raise HTTPException(400, "顶点格式应为 [x, y]")
    try:
        zid = store.add_zone(body.camera_id, body.name, body.polygon)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return {"id": zid}


@router.delete("/zones/{zid}", dependencies=[Depends(require_admin)])
def delete_zone(zid: int):
    store.delete_zone(zid)
    return {"ok": True}


# ---- rules ----
@router.get("/rules", dependencies=[Depends(require_read)])
def rules(camera_id: int | None = None):
    return store.rules(camera_id)


@router.post("/rules", dependencies=[Depends(require_admin)])
def add_rule(body: RuleCreate):
    if body.behavior not in ("enter_no_ppe", "enter", "exit", "occupancy_over"):
        raise HTTPException(400, "unknown behavior")
    if store.camera(body.camera_id) is None:  # 2026-10-30 审阅：指向不存在摄像头的规则静默失活
        raise HTTPException(404, "camera not found")
    if body.zone_id is not None:
        zone = store.zone(body.zone_id)
        if zone is None:
            raise HTTPException(404, "zone not found")
        if zone["camera_id"] != body.camera_id:  # 105 补丁 P1-1：跨摄像头规则静默失活，入库即拒
            raise HTTPException(400, f"区域 {body.zone_id} 属于摄像头 #{zone['camera_id']}，"
                                     f"不能挂到摄像头 #{body.camera_id}")
    rid = store.add_rule(body.model_dump())
    return {"id": rid}


@router.delete("/rules/{rid}", dependencies=[Depends(require_admin)])
def delete_rule(rid: int):
    store.delete_rule(rid)
    return {"ok": True}


# ---- events ----
@router.get("/events", dependencies=[Depends(require_read)])
def events(limit: int = 50, camera_id: int | None = None, level: int | None = None,
           status: str | None = None, offset: int = 0,
           start_ts: str | None = None, end_ts: str | None = None):
    limit = max(1, min(limit, 200))  # 任务19：上限钳制
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    return [_event_row(r) for r in
            store.events(limit=limit, camera_id=camera_id, level=level, status=status,
                         offset=offset, start_ts=start_ts, end_ts=end_ts)]


@router.get("/events/count", dependencies=[Depends(require_read)])
def events_count(camera_id: int | None = None, level: int | None = None,
                 status: str | None = None,
                 start_ts: str | None = None, end_ts: str | None = None):
    """与 events 相同过滤条件的总数（任务19；分页用）。"""
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    return {"count": store.count_events(camera_id=camera_id, level=level, status=status,
                                        start_ts=start_ts, end_ts=end_ts)}


def _normalize_ts_range(start_ts: str | None, end_ts: str | None):
    """纯日期输入补时间部分（含边界）：start→T00:00:00，end→T23:59:59。"""
    if start_ts and len(start_ts) == 10:
        start_ts += "T00:00:00"
    if end_ts and len(end_ts) == 10:
        end_ts += "T23:59:59"
    return start_ts, end_ts


@router.post("/events/{eid}/status", dependencies=[Depends(require_admin)])
def set_event_status(eid: int, body: dict):
    """事件处置（任务12）：status ∈ acked|false_positive|ignored。

    标记误报时把截图**复制**到 server/data/annotation_pool/（不删原事件）——
    数据飞轮的误报样本池，供 preannotate.py 直接喂给 Roboflow。
    """
    status = (body or {}).get("status", "")
    if status not in EVENT_STATUSES or status == "new":
        raise HTTPException(400, f"status 必须是 {'|'.join(EVENT_STATUSES[1:])}")
    row = store.event_by_id(eid)  # 2026-10-30 审阅：不再全表拉取
    if row is None:
        raise HTTPException(404, "event not found")
    if not store.set_event_status(eid, status, handled_by=(body or {}).get("handled_by", "admin")):
        raise HTTPException(404, "event not found")
    if status == "false_positive" and row.get("snapshot") and os.path.exists(row["snapshot"]):
        pool = settings.annotation_pool_dir
        pool.mkdir(parents=True, exist_ok=True)
        ts = row["ts"].replace(":", "").replace("-", "").replace("T", "_")
        dst = pool / f"ev{eid}_{ts}_cam{row['camera_id']}_{row['type']}.jpg"
        try:
            shutil.copy2(row["snapshot"], dst)  # 复制不移动：原事件截图保留
        except OSError as e:
            print(f"[annotation-pool] 复制失败 ev#{eid}: {e}")
    return {"ok": True, "status": status}


@router.get("/events/{eid}/snapshot", dependencies=[Depends(require_read)])
def event_snapshot(eid: int):
    path = store.event_snapshot(eid)
    if not path:
        raise HTTPException(404, "no snapshot")
    import os
    if not os.path.exists(path):
        raise HTTPException(404, "snapshot file missing")
    return FileResponse(path, media_type="image/jpeg")


@router.get("/events/{eid}/video", dependencies=[Depends(require_read)])
def event_video(eid: int):
    path = store.event_video(eid)
    if not path:
        raise HTTPException(404, "no video")
    import os
    if not os.path.exists(path):
        raise HTTPException(404, "video file missing")
    return FileResponse(path, media_type="video/mp4")


# ---- face（合规敏感：默认关闭，见 app/face.py）----
def _face_guard():
    if not settings.face_enabled:
        raise HTTPException(403, "人脸比对功能未开启（LS_FACE_ENABLED=false）。开启方式与合规要求见 README。")
    if not face_client.configured:
        raise HTTPException(503, "人脸服务未配置（缺少 LS_COMPREFACE_API_KEY）")


@router.get("/face/status")
def face_status():
    """人脸功能状态（只读展示用；开关只能改配置重启，不提供界面开关——防误开）。"""
    return {"enabled": settings.face_enabled, "configured": face_client.configured,
            "threshold": settings.face_threshold}


@router.post("/face/persons", dependencies=[Depends(require_admin)])
async def face_register(subject: str = Form(...), consent: bool = Form(...),
                        photo: UploadFile = File(...)):
    """注册人员：照片转存 CompreFace，同意记录落本地库。功能关闭时 403。"""
    _face_guard()
    if not consent:
        raise HTTPException(400, "注册人员必须 consent=true（《人脸识别技术应用安全管理办法》要求留痕）")
    data = await photo.read()
    if not data:
        raise HTTPException(400, "照片为空")
    try:
        face_client.add_person(subject, data)
    except Exception as e:  # noqa: BLE001
        raise HTTPException(502, f"CompreFace 注册失败: {e}")
    store.add_face_consent(subject)
    return {"ok": True, "subject": subject, "consent_ts": store.face_consent_ts(subject)}


@router.get("/face/persons", dependencies=[Depends(require_read)])
def face_persons():
    _face_guard()
    consents = {r["subject"]: r["ts"] for r in store.face_consents()}
    try:
        persons = face_client.list_persons()
    except Exception as e:  # noqa: BLE001
        raise HTTPException(502, f"CompreFace 人员列表获取失败: {e}")
    return [{"subject": s, "consent_ts": consents.get(s)} for s in persons]


@router.delete("/face/persons/all", dependencies=[Depends(require_admin)])
def face_clear_all():
    """一键清除：人员库 + 同意记录（合规要求）。"""
    _face_guard()
    errors = []
    for s in face_client.list_persons():
        try:
            face_client.delete_person(s)
        except Exception as e:  # noqa: BLE001
            errors.append(f"{s}: {e}")
    store.clear_face_consents()
    return {"ok": not errors, "errors": errors}


@router.delete("/face/persons/{subject}", dependencies=[Depends(require_admin)])
def face_delete(subject: str):
    _face_guard()
    try:
        face_client.delete_person(subject)
    except Exception as e:  # noqa: BLE001
        raise HTTPException(502, f"CompreFace 删除失败: {e}")
    return {"ok": True}


# ---- alert deliveries（任务13：推送可靠性）----
@router.get("/alerts/status", dependencies=[Depends(require_read)])
def alerts_status():
    """渠道状态页数据（任务23A，只读）：configured 判定不含敏感值；deliveries 汇总自投递表。"""
    configured_map = {
        "wecom": bool(settings.wecom_webhook),
        "dingtalk": bool(settings.dingtalk_webhook),
        "feishu": bool(settings.feishu_webhook),
        "serverchan": bool((settings.serverchan_sendkey or "").strip()),
        "pushplus": bool((settings.pushplus_token or "").strip()),
        "qmsg": bool((settings.qmsg_key or "").strip() and (settings.qmsg_qq or "").strip()),
        "email": bool(settings.smtp_host and settings.alert_email_to),
        "webhook": bool((settings.webhook_url or "").strip()),  # 任务67
    }
    by_ch = dispatcher.store.delivery_counts_by_channel() if dispatcher.store else {}
    channels = []
    for ch in dispatcher.channels:
        name = getattr(ch, "name", type(ch).__name__)
        channels.append({"name": name,
                         "configured": configured_map.get(name, False),
                         "deliveries": by_ch.get(name, {"ok": 0, "failed": 0, "pending": 0})})
    return {"channels": channels, "test_supported": True}


@router.get("/alert-deliveries", dependencies=[Depends(require_read)])
def alert_deliveries(status: str | None = None, limit: int = 50):
    if not dispatcher.store:
        raise HTTPException(503, "store 未绑定（服务未完成启动）")
    return dispatcher.store.deliveries(status=status, limit=max(1, min(limit, 200)))


@router.post("/alert-deliveries/{did}/retry", dependencies=[Depends(require_admin)])
def alert_delivery_retry(did: int):
    """死信手动重发：按原渠道、原事件文案重投（重新走三次退避）。"""
    if not dispatcher.store:
        raise HTTPException(503, "store 未绑定")
    rec = dispatcher.store.delivery(did)
    if rec is None:
        raise HTTPException(404, "delivery not found")
    ch = next((c for c in dispatcher.channels if getattr(c, "name", "") == rec["channel"]), None)
    if ch is None:
        raise HTTPException(400, f"未知渠道: {rec['channel']}")
    if rec["event_id"]:
        row = store.event_by_id(rec["event_id"])  # 2026-10-30 审阅：按 id 单查（原全表拉取）
        if row is None:
            raise HTTPException(404, "原事件已不存在")
        text = render_alert(row)
        level = row["level"]
    else:  # event_id=0 为测试消息
        level, text = 1, "【重发测试】LabSentinel 渠道自检消息。"
    dispatcher.store.update_delivery(did, "pending", rec["attempts"], None)
    dispatcher._pool.submit(dispatcher._deliver_with_retry, ch, rec["event_id"], level, text)
    return {"ok": True, "retrying": did}


@router.post("/alert-test", dependencies=[Depends(require_admin)])
def alert_test():
    """渠道健康自检：向所有已配置渠道发测试消息（event_id=0，不落 events）。部署验收用。"""
    return dispatcher.send_test()


# ---- maintenance（任务14：数据保留/清理）----
@router.post("/maintenance/cleanup", dependencies=[Depends(require_admin)])
def maintenance_cleanup():
    """手动触发一轮清理（幂等）。返回 {deleted_events, deleted_files, freed_mb, disk_free_gb}。"""
    from . import retention
    return retention.run_cleanup(store)


@router.get("/diagnostics/summary", dependencies=[Depends(require_read)])
def diagnostics_summary():
    """诊断聚合（任务61）：只读、可公开的聚合信息；错误取自进程内 RingHandler。"""
    from . import diagnostics
    return diagnostics.summary(store)


@router.get("/maintenance/preview", dependencies=[Depends(require_read)])
def maintenance_preview():
    """清理预演（任务68）：只读，返回将删除的事件数/字节与释放预估，绝不删除。"""
    from . import retention
    return retention.preview_cleanup(store)


@router.get("/maintenance/status", dependencies=[Depends(require_read)])
def maintenance_status():
    """维护状态（任务14/68）：清理结果 + 磁盘水位/数据体量/增速预估（估算值已标注）。"""
    import os
    from . import retention

    def _core_bytes() -> int:
        """db + 截图/视频/标注池四目录的递归大小。"""
        total = 0
        roots = [settings.data_dir / "labsentinel.db", settings.snapshot_dir,
                 settings.video_dir, settings.annotation_pool_dir]
        for r in roots:
            if r.is_file():
                total += r.stat().st_size
            elif r.is_dir():
                for dirpath, _d, files in os.walk(r):
                    for f in files:
                        try:
                            total += os.path.getsize(os.path.join(dirpath, f))
                        except OSError:
                            pass
        return total

    def _data_bytes() -> tuple[int, bool]:
        """全 data_dir 递归大小；超 2 万文件退化为四核心目录（防大目录卡 API）。"""
        total, n = 0, 0
        for dirpath, _d, files in os.walk(settings.data_dir):
            for f in files:
                n += 1
                if n > 20000:
                    return _core_bytes(), True
                try:
                    total += os.path.getsize(os.path.join(dirpath, f))
                except OSError:
                    pass
        return total, False

    data_bytes, capped = _data_bytes()
    free_gb = retention._disk_free_gb()
    try:
        total_gb = shutil.disk_usage(str(settings.data_dir)).total / 1e9
        free_pct = round(free_gb / total_gb * 100, 1) if total_gb else None
    except OSError:
        total_gb, free_pct = None, None
    # forecast_days（粗略估算）：近 7 天事件关联文件的日均字节 → 剩余空间可撑天数
    forecast_days = None
    try:
        with store._conn() as c:
            rows = c.execute(
                "SELECT snapshot, video FROM events WHERE ts >= ? LIMIT 5000",
                ((datetime.now() - timedelta(days=7)).isoformat(timespec="seconds"),)).fetchall()
        week_bytes = 0
        for r in rows:
            for col in ("snapshot", "video"):
                p = r[col]
                if p and os.path.isfile(p):
                    try:
                        week_bytes += os.path.getsize(p)
                    except OSError:
                        pass
        if week_bytes > 0:
            per_day_gb = (week_bytes / 7) / 1e9
            if per_day_gb > 0:
                forecast_days = round(free_gb / per_day_gb)
    except Exception:  # noqa: BLE001  预估失败不阻塞状态端点
        forecast_days = None
    return {"last_cleanup": retention.last_result(),
            "disk_free_gb": round(free_gb, 2),
            "disk_total_gb": round(total_gb, 2) if total_gb else None,
            "disk_free_pct": free_pct,
            "data_bytes": data_bytes,
            "data_bytes_capped": capped,
            "low_disk": free_gb < settings.disk_min_free_gb,
            "forecast_days": forecast_days,  # 估算值：近7天日均增速推算，仅供预警参考
            "retention": {"events_days": settings.retention_events_days,
                          "pool_days": settings.retention_pool_days,
                          "disk_min_free_gb": settings.disk_min_free_gb}}


# ---- export（任务15：报表导出）----
@router.get("/export/events.csv", dependencies=[Depends(require_read)])
def export_events_csv(days: int = 30, camera_id: int | None = None, status: str | None = None):
    """事件 CSV（UTF-8 带 BOM，Excel 中文不乱码；生成器流式返回）。"""
    from .export import iter_events_csv

    def gen():
        yield "\ufeff"  # UTF-8 BOM
        for row in iter_events_csv(store, days=days, camera_id=camera_id, status=status):
            buf = io.StringIO()
            csv.writer(buf).writerow(row)
            yield buf.getvalue()

    filename = f"events_{datetime.now().strftime('%Y%m%d_%H%M')}.csv"
    return StreamingResponse(gen(), media_type="text/csv; charset=utf-8",
                             headers={"Content-Disposition": f"attachment; filename={filename}"})


@router.get("/export/stats.csv", dependencies=[Depends(require_read)])
def export_stats_csv(days: int = 30, camera_id: int | None = None):
    from .export import stats_csv
    buf = io.StringIO()
    w = csv.writer(buf)
    for row in stats_csv(store, days=days, camera_id=camera_id):
        w.writerow(row)
    return Response(
        content="\ufeff" + buf.getvalue(), media_type="text/csv; charset=utf-8",
        headers={"Content-Disposition":
                 f"attachment; filename=stats_{datetime.now().strftime('%Y%m%d')}.csv"})


@router.get("/export/monthly.pdf", dependencies=[Depends(require_read)])
def export_monthly_pdf(month: str):
    """月报 PDF（不含截图/人脸，合规）。month=2026-10。"""
    from .export import monthly_pdf
    try:
        datetime.strptime(month, "%Y-%m")
    except ValueError:
        raise HTTPException(400, "month 必须是 YYYY-MM 格式")
    try:
        data = monthly_pdf(store, month)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return Response(data, media_type="application/pdf",
                    headers={"Content-Disposition":
                             f"attachment; filename=monthly_{month}.pdf"})


@router.get("/export/evidence.zip", dependencies=[Depends(require_read)])
def export_evidence_zip(camera_id: int | None = None, level: int | None = None,
                        status: str | None = None,
                        start_ts: str | None = None, end_ts: str | None = None):
    """事件证据包 ZIP（任务33）：events.csv + 截图 + 视频 + README；超限 400。"""
    from .export import build_evidence_zip
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    try:
        data, _stats = build_evidence_zip(store, settings.data_dir, camera_id=camera_id,
                                          level=level, status=status,
                                          start_ts=start_ts, end_ts=end_ts)
    except ValueError as e:
        raise HTTPException(400, str(e))
    filename = f"labsentinel_evidence_{datetime.now().strftime('%Y%m%d_%H%M')}.zip"
    return Response(data, media_type="application/zip",
                    headers={"Content-Disposition": f"attachment; filename={filename}"})


# ---- stats ----
@router.get("/stats/daily", dependencies=[Depends(require_read)])
def stats_daily(days: int = 7, camera_id: int | None = None, status: str | None = None,
                start_ts: str | None = None, end_ts: str | None = None):
    try:  # 2026-10-30 审阅：非法日期统一 400（原 ValueError 冒泡成 500，与 hourly 口径不一致）
        start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
        return stats_daily_query(store, days=days, camera_id=camera_id, status=status,
                                 start_ts=start_ts, end_ts=end_ts)
    except ValueError as e:
        raise HTTPException(400, f"日期格式错误: {e}")


@router.get("/stats/weekly", dependencies=[Depends(require_read)])
def stats_weekly(weeks: int = 4, camera_id: int | None = None, status: str | None = None):
    """按 ISO 周聚合（周一为起点），最近 N 周（任务21）。"""
    return stats_weekly_query(store, weeks=weeks, camera_id=camera_id, status=status)


@router.get("/stats/summary", dependencies=[Depends(require_read)])
def stats_summary(days: int = 30, camera_id: int | None = None, status: str | None = None):
    """汇总卡片：总数/分级/分类型/Top 事件日（任务21）。"""
    return stats_summary_query(store, days=days, camera_id=camera_id, status=status)


@router.get("/stats/hourly", dependencies=[Depends(require_read)])
def stats_hourly(date: str, camera_id: int | None = None):
    try:
        return stats_hourly_query(store, date, camera_id=camera_id)
    except ValueError as e:
        raise HTTPException(400, str(e))


# ---- websocket ----
async def ws_events(ws: WebSocket):
    # 任务16：启用鉴权时 WS 用 query ?token=（URL 传 token 有日志泄漏风险，文档已注明）
    if not _token_ok(ws.query_params.get("token")):
        await ws.close(code=WS_AUTH_CODE)
        return
    await dispatcher.hub.connect(ws)
    try:
        while True:
            await ws.receive_text()  # 忽略客户端心跳
    except WebSocketDisconnect:
        dispatcher.hub.disconnect(ws)
