"""T-505 第一步（纯搬移）：stats / export 路由。

从 api.py 原样搬出，行为不变。共享状态与 helper 通过延迟 `from . import api` 引用，
避免在导入期形成 api ↔ routers 循环依赖（api.py 在模块末尾 include_router 本模块）。
"""
import csv
import io
from datetime import datetime

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import Response, StreamingResponse

from . import _context as ctx
from ..config import settings
from ..stats import (
    daily as stats_daily_query,
    hourly as stats_hourly_query,
    quality as stats_quality_query,
    summary as stats_summary_query,
    weekly as stats_weekly_query,
)

router = APIRouter()


# ---- export（任务15：报表导出）----
@router.get("/export/events.csv", dependencies=[Depends(ctx.read_guard)])
def export_events_csv(days: int = 30, camera_id: int | None = None, status: str | None = None,
                      q: str | None = None, level: int | None = None,
                      start_ts: str | None = None, end_ts: str | None = None):
    """事件 CSV（UTF-8 带 BOM，Excel 中文不乱码；生成器流式返回）。"""
    from ..export import iter_events_csv

    def gen():
        yield "\ufeff"  # UTF-8 BOM
        for row in iter_events_csv(ctx.store(), days=days, camera_id=camera_id, status=status,
                                   q=q, level=level, start_ts=start_ts, end_ts=end_ts):
            buf = io.StringIO()
            csv.writer(buf).writerow(row)
            yield buf.getvalue()

    filename = f"events_{datetime.now().strftime('%Y%m%d_%H%M')}.csv"
    return StreamingResponse(gen(), media_type="text/csv; charset=utf-8",
                             headers={"Content-Disposition": f"attachment; filename={filename}"})


@router.get("/export/stats.csv", dependencies=[Depends(ctx.read_guard)])
def export_stats_csv(days: int = 30, camera_id: int | None = None):
    from ..export import stats_csv
    buf = io.StringIO()
    w = csv.writer(buf)
    for row in stats_csv(ctx.store(), days=days, camera_id=camera_id):
        w.writerow(row)
    return Response(
        content="\ufeff" + buf.getvalue(), media_type="text/csv; charset=utf-8",
        headers={"Content-Disposition":
                 f"attachment; filename=stats_{datetime.now().strftime('%Y%m%d')}.csv"})


@router.get("/export/monthly.pdf", dependencies=[Depends(ctx.read_guard)])
def export_monthly_pdf(month: str):
    """月报 PDF（不含截图/人脸，合规）。month=2026-10。"""
    from ..export import monthly_pdf
    try:
        datetime.strptime(month, "%Y-%m")
    except ValueError:
        raise HTTPException(400, "month 必须是 YYYY-MM 格式")
    try:
        data = monthly_pdf(ctx.store(), month)
    except ImportError as e:
        raise HTTPException(503, f"PDF 月报依赖缺失（reportlab），请安装 server/requirements.txt 中的 reportlab: {e}")
    except ValueError as e:
        raise HTTPException(400, str(e))
    return Response(data, media_type="application/pdf",
                    headers={"Content-Disposition":
                             f"attachment; filename=monthly_{month}.pdf"})


@router.get("/events/{eid}/evidence.zip", dependencies=[Depends(ctx.read_guard)])
def event_evidence_zip(eid: int):
    """单事件证据包（任务103）：event.json+截图+视频+README；缺失文件跳过并写明，不 500。"""
    from ..export import build_event_evidence
    try:
        out = build_event_evidence(ctx.store(), settings.data_dir, eid)
    except ValueError as e:
        raise HTTPException(400, str(e))
    if out is None:
        raise HTTPException(404, "event not found")
    data, meta = out
    return Response(
        content=data, media_type="application/zip",
        headers={"Content-Disposition": f'attachment; filename="event-{eid}-evidence.zip"',
                 "X-Evidence-Files": str(meta["files"])})


@router.get("/export/evidence.zip", dependencies=[Depends(ctx.read_guard)])
def export_evidence_zip(camera_id: int | None = None, level: int | None = None,
                        status: str | None = None,
                        start_ts: str | None = None, end_ts: str | None = None):
    """事件证据包 ZIP（任务33）：events.csv + 截图 + 视频 + README；超限 400。"""
    from ..export import build_evidence_zip
    start_ts, end_ts = ctx.normalize_ts_range(start_ts, end_ts)
    try:
        data, _stats = build_evidence_zip(ctx.store(), settings.data_dir, camera_id=camera_id,
                                          level=level, status=status,
                                          start_ts=start_ts, end_ts=end_ts)
    except ValueError as e:
        raise HTTPException(400, str(e))
    filename = f"labsentinel_evidence_{datetime.now().strftime('%Y%m%d_%H%M')}.zip"
    return Response(data, media_type="application/zip",
                    headers={"Content-Disposition": f"attachment; filename={filename}"})


# ---- stats ----
@router.get("/stats/daily", dependencies=[Depends(ctx.read_guard)])
def stats_daily(days: int = 7, camera_id: int | None = None, status: str | None = None,
                start_ts: str | None = None, end_ts: str | None = None):
    try:  # 2026-10-30 审阅：非法日期统一 400（原 ValueError 冒泡成 500，与 hourly 口径不一致）
        start_ts, end_ts = ctx.normalize_ts_range(start_ts, end_ts)
        return stats_daily_query(ctx.store(), days=days, camera_id=camera_id, status=status,
                                 start_ts=start_ts, end_ts=end_ts)
    except ValueError as e:
        raise HTTPException(400, f"日期格式错误: {e}")


@router.get("/stats/weekly", dependencies=[Depends(ctx.read_guard)])
def stats_weekly(weeks: int = 4, camera_id: int | None = None, status: str | None = None):
    """按 ISO 周聚合（周一为起点），最近 N 周（任务21）。"""
    return stats_weekly_query(ctx.store(), weeks=weeks, camera_id=camera_id, status=status)


@router.get("/stats/summary", dependencies=[Depends(ctx.read_guard)])
def stats_summary(days: int = 30, camera_id: int | None = None, status: str | None = None):
    """汇总卡片：总数/分级/分类型/Top 事件日（任务21）。"""
    return stats_summary_query(ctx.store(), days=days, camera_id=camera_id, status=status)


@router.get("/stats/quality", dependencies=[Depends(ctx.read_guard)])
def stats_quality(days: int = 7, camera_id: int | None = None):
    """处置质量统计（任务102）：误报率=fp/(acked+fp+ignored)，分母为已处置事件；无已处置=0。"""
    if not 1 <= days <= 365:
        raise HTTPException(400, "days must be 1-365")
    return stats_quality_query(ctx.store(), days=days, camera_id=camera_id)


@router.get("/stats/hourly", dependencies=[Depends(ctx.read_guard)])
def stats_hourly(date: str, camera_id: int | None = None):
    try:
        return stats_hourly_query(ctx.store(), date, camera_id=camera_id)
    except ValueError as e:
        raise HTTPException(400, str(e))
