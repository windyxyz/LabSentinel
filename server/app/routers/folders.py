"""T-505 第五步（纯搬移）：图片文件夹识别路由（任务121）。

从 api.py 原样搬出，行为不变；共享对象经 routers/_context.py 惰性代理
（路径安全 helper 保留在 api.py，图像处理工具在 folders.py）。
"""
import csv
import io
import mimetypes
import os
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import FileResponse, Response

from . import _context as ctx
from ..config import settings
from ..folders import image_exts, thumbnail_bytes

router = APIRouter()


@router.get("/folder-images/summary", dependencies=[Depends(ctx.read_guard)])
def folder_images_summary(days: int = 7, camera_id: int | None = None):
    """图片识别汇总（近 N 天，按摄像头）：汇报卡片与"关键/违规"趋势。"""
    days = max(1, min(int(days), 3650))
    cams = {c["id"]: c for c in ctx.store().cameras()}
    rows = ctx.store().folder_image_summary(days=days, camera_id=camera_id)
    items, seen = [], set()
    totals = {"total": 0, "key": 0, "violation": 0, "persons": 0, "errors": 0}
    for r in rows:
        cam = cams.get(r["camera_id"], {})
        item = {
            "camera_id": r["camera_id"], "camera_name": cam.get("name"),
            "status": cam.get("status"), "total": int(r["total"] or 0),
            "key": int(r["key_n"] or 0), "violation": int(r["violation_n"] or 0),
            "persons": int(r["persons_sum"] or 0), "in_zone": int(r["in_zone_sum"] or 0),
            "errors": int(r["error_n"] or 0),
            "last_image_ts": r["last_image_ts"], "last_processed_ts": r["last_processed_ts"],
        }
        items.append(item)
        seen.add(r["camera_id"])
        for k in totals:
            totals[k] += item[k]
    for c in cams.values():  # 尚无识别记录的 folder 摄像头也列出（UI 显示"等待图片"）
        if c["type"] != "folder" or c["id"] in seen:
            continue
        if camera_id is not None and c["id"] != camera_id:
            continue
        items.append({"camera_id": c["id"], "camera_name": c["name"], "status": c["status"],
                      "total": 0, "key": 0, "violation": 0, "persons": 0, "in_zone": 0,
                      "errors": 0, "last_image_ts": None, "last_processed_ts": None})
    items.sort(key=lambda x: x["camera_id"])
    return {"days": days, "items": items, "totals": totals}


@router.get("/folder-images", dependencies=[Depends(ctx.read_guard)])
def folder_images(camera_id: int | None = None, key: int | None = None,
                  violation: int | None = None, start_ts: str | None = None,
                  end_ts: str | None = None, error_only: int = 0,
                  limit: int = 60, offset: int = 0):
    """关键图片列表（默认全部，key=1 只看有人的图，violation=1 只看违规图）。"""
    limit = max(1, min(int(limit), 500))
    offset = max(0, int(offset))
    rows = ctx.store().folder_images(camera_id=camera_id, key=key, violation=violation,
                                     start_ts=start_ts, end_ts=end_ts, error_only=bool(error_only),
                                     limit=limit, offset=offset)
    total = ctx.store().count_folder_images(camera_id=camera_id, key=key, violation=violation,
                                            start_ts=start_ts, end_ts=end_ts,
                                            error_only=bool(error_only))
    names = {c["id"]: c["name"] for c in ctx.store().cameras()}
    return {"items": [ctx.folder_row_out(r) for r in rows], "total": total,
            "limit": limit, "offset": offset,
            "camera_names": names}


@router.get("/folder-images/export.csv", dependencies=[Depends(ctx.read_guard)])
def export_folder_images_csv(camera_id: int | None = None, key: int | None = None,
                             violation: int | None = None, start_ts: str | None = None,
                             end_ts: str | None = None, error_only: int = 0):
    """按当前筛选导出图片识别记录 CSV（UTF-8 BOM，Excel 直接打开）。最多 5 万行。"""
    rows: list[dict] = []
    offset = 0
    while len(rows) < 50000:
        batch = ctx.store().folder_images(camera_id=camera_id, key=key, violation=violation,
                                          start_ts=start_ts, end_ts=end_ts,
                                          error_only=bool(error_only), limit=500, offset=offset)
        rows.extend(batch)
        if len(batch) < 500:
            break
        offset += 500
    names = {c["id"]: c["name"] for c in ctx.store().cameras()}
    buf = io.StringIO()
    w = csv.writer(buf)
    w.writerow(["摄像头", "图片时间", "识别时间", "文件名", "源路径", "人数", "在区",
                "未穿", "已穿", "命中区域", "关键", "违规", "违规明细", "关联事件", "读取错误"])
    for r in rows:
        w.writerow([ctx.csv_cell(v) for v in [
            names.get(r["camera_id"], f"#{r['camera_id']}"), r["ts"], r["processed_ts"],
            r["name"] or Path(str(r["path"])).name, r["path"], r["persons"], r["in_zone"],
            r["ppe_no"], r["ppe_wearing"], "、".join(r["zones"] or []),
            "是" if r["key"] else "否", "是" if r["violation"] else "否",
            "；".join(f"{v.get('type')}{('/' + v['zone']) if v.get('zone') else ''}"
                      f"{('/数量' + str(v['count'])) if v.get('count') is not None else ''}"
                      for v in (r["violations"] or [])),
            r["event_id"] or "", r["error"] or "",
        ]])
    body = "\ufeff" + buf.getvalue()
    return Response(content=body.encode("utf-8"), media_type="text/csv; charset=utf-8",
                    headers={"Content-Disposition":
                             'attachment; filename="labsentinel-folder-images.csv"'})


@router.get("/folder-images/{iid}", dependencies=[Depends(ctx.read_guard)])
def folder_image_detail(iid: int):
    row = ctx.store().folder_image(iid)
    if row is None:
        raise HTTPException(404, "image record not found")
    return ctx.folder_row_out(row)


@router.get("/folder-images/{iid}/file", dependencies=[Depends(ctx.read_guard)])
def folder_image_file(iid: int):
    row = ctx.store().folder_image(iid)
    if row is None:
        raise HTTPException(404, "image record not found")
    p = ctx.folder_readable_path(row)
    mt = mimetypes.guess_type(str(p))[0] or "application/octet-stream"
    return FileResponse(p, media_type=mt, headers={"Cache-Control": "private, max-age=3600"})


@router.get("/folder-images/{iid}/thumb", dependencies=[Depends(ctx.read_guard)])
def folder_image_thumb(iid: int):
    """列表缩略图（480px，磁盘缓存）；原图走 /file，避免首屏拉一堆 4K 截图。"""
    row = ctx.store().folder_image(iid)
    if row is None:
        raise HTTPException(404, "image record not found")
    p = ctx.folder_readable_path(row)
    data = thumbnail_bytes(str(p), iid)
    if data is None:
        raise HTTPException(415, "缩略图生成失败（图片损坏或格式不支持）")
    return Response(content=data, media_type="image/jpeg",
                    headers={"Cache-Control": "private, max-age=3600"})


@router.post("/folders/{cid}/scan", dependencies=[Depends(ctx.admin_guard)])
def folder_scan(cid: int):
    """立即扫描：运行中的文件夹源唤醒一轮；未运行则启动该路接入。"""
    cam = ctx.store().camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    if cam["type"] != "folder":
        raise HTTPException(400, "该摄像头不是图片文件夹源")
    if ctx.manager().wake_camera(cid):
        return {"ok": True, "started": False, "msg": "已触发立即扫描"}
    ctx.manager().start_camera(cam)
    return {"ok": True, "started": True, "msg": "图片文件夹接入已启动"}
