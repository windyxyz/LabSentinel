"""T-505 第六步（纯搬移）：events 列表/计数/处置/快照/视频/时间线 路由。

从 api.py 原样搬出，行为不变；共享对象经 routers/_context.py 惰性代理
（_event_row/_triage_one 等纯函数随本模块搬移，api.py 留兼容别名）。
"""
import os
import shutil

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse

from . import _context as ctx
from ..config import settings
from ..models import BulkStatusBody, EventStatusBody

router = APIRouter()


def _normalize_ts_range(start_ts: str | None, end_ts: str | None):
    """纯日期输入补时间部分（含边界）：start→T00:00:00，end→T23:59:59。"""
    if start_ts and len(start_ts) == 10:
        start_ts += "T00:00:00"
    if end_ts and len(end_ts) == 10:
        end_ts += "T23:59:59"
    return start_ts, end_ts


def _triage_one(eid: int, status: str, handled_by: str) -> str:
    """单条处置核心（任务83 抽出供批量复用，与单条端点同一套状态规则）。

    返回 "updated"（含 false_positive 截图入池）或 "skipped"（已是目标状态/不存在）。
    """
    store = ctx.store()
    row = store.event_by_id(eid)
    if row is None:
        return "missing"
    if row["status"] == status:
        return "skipped"
    if not store.set_event_status(eid, status, handled_by=handled_by):
        return "missing"
    if status == "false_positive" and row.get("snapshot") and os.path.exists(row["snapshot"]):
        pool = settings.annotation_pool_dir
        pool.mkdir(parents=True, exist_ok=True)
        ts = row["ts"].replace(":", "").replace("-", "").replace("T", "_")
        dst = pool / f"ev{eid}_{ts}_cam{row['camera_id']}_{row['type']}.jpg"
        try:
            shutil.copy2(row["snapshot"], dst)  # 复制不移动：原事件截图保留
        except OSError as e:
            print(f"[annotation-pool] 复制失败 ev#{eid}: {e}")
    return "updated"


@router.get("/events", dependencies=[Depends(ctx.read_guard)])
def events(limit: int = 50, camera_id: int | None = None, level: int | None = None,
           status: str | None = None, offset: int = 0,
           start_ts: str | None = None, end_ts: str | None = None,
           q: str | None = None, ai_verdict: str | None = None):
    if q is not None and len(q.strip()) > 100:  # 任务84：>100 字符拒绝；空白=不过滤
        raise HTTPException(400, "q 长度须 ≤ 100 字符")
    ctx.check_ai_verdict(ai_verdict)  # 任务132
    limit = max(1, min(limit, 200))  # 任务19：上限钳制
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    return [ctx.event_row(r) for r in
            ctx.store().events(limit=limit, camera_id=camera_id, level=level, status=status,
                               offset=offset, start_ts=start_ts, end_ts=end_ts, q=q,
                               ai_verdict=ai_verdict)]


@router.get("/events/count", dependencies=[Depends(ctx.read_guard)])
def events_count(camera_id: int | None = None, level: int | None = None,
                 status: str | None = None,
                 start_ts: str | None = None, end_ts: str | None = None,
                 q: str | None = None, ai_verdict: str | None = None):
    """与 events 相同过滤条件的总数（任务19；分页用；任务84 q 关键词）。"""
    if q is not None and len(q.strip()) > 100:
        raise HTTPException(400, "q 长度须 ≤ 100 字符")
    ctx.check_ai_verdict(ai_verdict)  # 任务132
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    return {"count": ctx.store().count_events(camera_id=camera_id, level=level, status=status,
                                              start_ts=start_ts, end_ts=end_ts, q=q,
                                              ai_verdict=ai_verdict)}


@router.post("/events/{eid}/status", dependencies=[Depends(ctx.triage_guard)])  # T-620
def set_event_status(eid: int, body: EventStatusBody):
    """事件处置（任务12）：status ∈ acked|false_positive|ignored。

    标记误报时把截图**复制**到 annotation_pool/（不删原事件）——数据飞轮误报样本池。
    """
    r = _triage_one(eid, body.status, body.handled_by)
    if r == "missing":
        raise HTTPException(404, "event not found")
    return {"ok": True, "status": body.status}


@router.post("/events/bulk-status", dependencies=[Depends(ctx.admin_guard)])
def bulk_event_status(body: BulkStatusBody):
    """批量处置（任务83）：逐条短事务复用单条逻辑；missing 容错；单条失败不中断整批。"""
    status = body.status
    ids = sorted(set(body.ids))
    handled_by = body.handled_by
    updated = skipped = 0
    missing, failed = [], []
    for eid in ids:
        try:
            r = _triage_one(eid, status, handled_by)
            if r == "updated":
                updated += 1
            elif r == "skipped":
                skipped += 1
            else:
                missing.append(eid)
        except Exception as e:  # noqa: BLE001  单条失败不中断整批
            failed.append({"id": eid, "error": str(e)[:120]})
    return {"updated": updated, "skipped": skipped, "missing": missing, "failed": failed}


# 单条/批量共用 _triage_one（防规则漂移）


@router.get("/events/{eid}/snapshot", dependencies=[Depends(ctx.read_guard)])
def event_snapshot(eid: int):
    path = ctx.store().event_snapshot(eid)
    if not path:
        raise HTTPException(404, "no snapshot")
    if not os.path.exists(path):
        raise HTTPException(404, "snapshot file missing")
    return FileResponse(path, media_type="image/jpeg")


@router.get("/events/{eid}/video", dependencies=[Depends(ctx.read_guard)])
def event_video(eid: int):
    path = ctx.store().event_video(eid)
    if not path:
        raise HTTPException(404, "no video")
    if not os.path.exists(path):
        raise HTTPException(404, "video file missing")
    return FileResponse(path, media_type="video/mp4")
