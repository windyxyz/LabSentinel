"""T-505 第四步（纯搬移）：cameras 分组/批量/USB/CRUD/webrtc/copy-config/status。

从 api.py 原样搬出，行为不变；共享对象经 routers/_context.py 惰性代理。
（/cameras 列表路由留在 api.py 原位置，保证 OpenAPI 路径顺序不漂移。）
"""
import os
import time
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException

from . import _context as ctx
from ..config import settings
from ..models import CameraCreate

router = APIRouter()


# ---- 分组与批量启停（任务111）----
@router.put("/cameras/{cid}/group", dependencies=[Depends(ctx.admin_guard)])
def set_camera_group(cid: int, body: dict):
    name = str(body.get("group_name") or "")
    if len(name) > 32:
        raise HTTPException(400, "group_name 最长 32 字符")
    if ctx.store().camera(cid) is None:
        raise HTTPException(404, "camera not found")
    ctx.store().set_camera_group(cid, name)
    return {"id": cid, "group_name": name}


@router.get("/camera-groups", dependencies=[Depends(ctx.read_guard)])
def camera_groups():
    return {"groups": ctx.store().camera_groups()}


@router.post("/cameras/bulk", dependencies=[Depends(ctx.admin_guard)])
def cameras_bulk(body: dict):
    """批量启停：{ids:[...], action:"start"|"stop"}。ids 去重、1-100 个；不删数据。"""
    ids = body.get("ids")
    action = body.get("action")
    if action not in ("start", "stop"):
        raise HTTPException(400, "action must be start|stop")
    if not isinstance(ids, list) or not ids or len(set(ids)) != len(ids) or len(ids) > 100:
        raise HTTPException(400, "ids 必须为去重非空列表且 ≤100 个")
    if not all(isinstance(i, int) and not isinstance(i, bool) for i in ids):
        raise HTTPException(400, "ids 必须是整数数组")
    missing, failed = [], []
    for cid in ids:
        cam = ctx.store().camera(cid)
        if cam is None:
            missing.append(cid)
            continue
        if ctx.manager() is None:
            failed.append({"id": cid, "error": "manager not ready"})
            continue
        try:
            if action == "start":
                ctx.manager().start_camera(cam)
            else:
                ctx.manager().stop_camera(cid)
        except Exception as e:  # noqa: BLE001
            failed.append({"id": cid, "error": repr(e)})
    return {"ok": [i for i in ids if i not in missing and i not in [f["id"] for f in failed]],
            "missing": missing, "failed": failed}


@router.get("/cameras/usb-devices", dependencies=[Depends(ctx.read_guard)])
def usb_devices(refresh: int = 0):
    """枚举本机 DirectShow USB 摄像头（任务60）。

    index 为枚举顺序（通常与 CAP_DSHOW 一致，驱动/插拔变化后可能改变，设备名更稳定）；
    失败一律 200 + devices=[] + 中文 error（前端回退手输索引）。
    """
    from .. import usb_devices as usb
    return usb.list_devices(refresh=bool(refresh))


@router.post("/cameras", dependencies=[Depends(ctx.admin_guard)])
def add_camera(body: CameraCreate):
    if body.type not in ("rtsp", "file", "usb", "folder"):
        raise HTTPException(400, "type must be rtsp|file|usb|folder")
    source = (body.source or "").strip()
    # T-103：source 白名单已在 CameraCreate 校验器执行（CameraCreate 返回规范化后的 source）
    if body.type == "folder":
        # T-111：允许根/系统目录黑名单（符号链接逃逸由 check_folder_root 的 resolve 处理）
        from ..sources import SourceError, check_folder_root
        try:
            source = check_folder_root(source, settings.folder_allowed_roots)
            data_root = os.path.normcase(str(Path(settings.data_dir).resolve()))
            if os.path.normcase(source) == data_root or \
                    os.path.normcase(source).startswith(data_root + os.sep):
                raise HTTPException(400, "图片文件夹不能使用程序数据目录（会造成证据图自激识别）")
        except SourceError as e:
            raise HTTPException(400, str(e))
    cid = ctx.store().add_camera(body.name, source, body.type, group_name=body.group_name)
    if body.autostart:
        ctx.manager().start_camera(ctx.store().camera(cid))
    from .. import go2rtc
    go2rtc.sync_streams(ctx.store())  # 实时画面：同步流媒体网关配置
    return ctx.store().camera(cid)


@router.put("/cameras/{cid}/analyzer", dependencies=[Depends(ctx.admin_guard)])
def set_camera_analyzer(cid: int, body: dict):
    """T-336：迁移期一键切换/回退单路分析器（python|rust）。运行中自动重启生效。"""
    impl = str(body.get("analyzer") or "")
    if impl not in ("python", "rust"):
        raise HTTPException(400, "analyzer 仅支持 python|rust")
    cam = ctx.store().camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    if (cam.get("analyzer") or "python") == impl:
        return {"id": cid, "analyzer": impl, "restarted": False}
    ctx.store().set_camera_analyzer(cid, impl)
    restarted = False
    if cam["status"] == "running":
        ctx.manager().stop_camera(cid)
        ctx.manager().start_camera(ctx.store().camera(cid))
        restarted = True
    return {"id": cid, "analyzer": impl, "restarted": restarted}


@router.post("/cameras/{cid}/start", dependencies=[Depends(ctx.admin_guard)])
def start_camera(cid: int):
    cam = ctx.store().camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    ctx.manager().start_camera(cam)
    return {"ok": True}


@router.delete("/cameras/{cid}", dependencies=[Depends(ctx.admin_guard)])
def delete_camera(cid: int):
    ctx.manager().stop_camera(cid)
    # 图片文件夹：删记录 + 派生证据/缩略图（原始截图永不删除）
    removed = ctx.store().delete_folder_images_by_camera(cid)
    for r in removed:
        # 事件仍引用同一证据副本时保留文件（事件保留期 180 天，查看证据不能 404）
        if not (r.get("snapshot") and ctx.store().event_snapshot_referenced(r["snapshot"])):
            ctx.unlink_data_file(r.get("snapshot"))
        ctx.unlink_data_file(str(Path(settings.data_dir) / "thumbs" / f"img_{r['id']}.jpg"))
    ctx.store().delete_camera(cid)
    from .. import go2rtc
    go2rtc.sync_streams(ctx.store())  # 实时画面：同步流媒体网关配置
    return {"ok": True}


@router.get("/cameras/{cid}/webrtc", dependencies=[Depends(ctx.read_guard)])
def camera_webrtc(cid: int):
    """T-102：返回经 FastAPI 鉴权代理的相对 WS 路径（浏览器不再直连 go2rtc :1984）。

    前端按 location 组装 ws/wss（http→ws、https→wss），令牌走 Sec-WebSocket-Protocol
    子协议（T-106），媒体面仍在 go2rtc :8555（webrtc.candidates 可配跨网段地址）。
    """
    cam = ctx.store().camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    if cam["type"] == "folder":
        raise HTTPException(400, "图片文件夹源没有实时流；请在「图片识别」页查看关键图片")
    return {"camera_id": cid, "ws_url": f"/ws/webrtc/{cid}"}


@router.post("/cameras/{cid}/copy-config", dependencies=[Depends(ctx.admin_guard)])
def copy_camera_config(cid: int, body: dict):
    """把源摄像头的区域/规则批量复制到多路目标（任务80）。

    先全量校验（源/目标存在、排除自身、目标去重）再写库；规则 zone_id 按**区域名**映射到
    目标（name_suffix 参与命名）；copy_zones=False 时规则若无同名目标区域则跳过并 warning
    （绝不指向源摄像头的区域）。不启动摄像头、不复制事件。
    """
    src = ctx.store().camera(cid)
    if src is None:
        raise HTTPException(404, "camera not found")
    targets = body.get("target_ids") or []
    if not targets:
        raise HTTPException(400, "target_ids 不能为空")
    targets = sorted(set(int(t) for t in targets))
    if cid in targets:
        raise HTTPException(400, "target_ids 不能包含源摄像头自身")
    missing = [t for t in targets if ctx.store().camera(t) is None]
    if missing:
        raise HTTPException(404, f"目标摄像头不存在: {missing}")
    copy_zones = bool(body.get("copy_zones", True))
    copy_rules = bool(body.get("copy_rules", True))
    suffix = str(body.get("name_suffix", "") or "")
    src_zones = ctx.store().zones(cid)
    src_rules = ctx.store().rules(cid)
    warnings: list[str] = []
    zones_created = zones_skipped = rules_created = rules_skipped = 0
    targets_out = []
    for tid in targets:
        zone_map: dict[str, int] = {}
        z0, r0 = zones_skipped, rules_skipped  # 每目标跳过数 = 前后差值
        tgt_zone_names = {z["name"]: z["id"] for z in ctx.store().zones(tid)}
        if copy_zones:
            for z in src_zones:
                new_name = z["name"] + suffix
                if new_name in tgt_zone_names:
                    zone_map[z["name"]] = tgt_zone_names[new_name]  # 已有同名：映射给规则用
                    zones_skipped += 1
                    continue
                zid = ctx.store().add_zone(tid, new_name, z["polygon"])
                tgt_zone_names[new_name] = zid
                zone_map[z["name"]] = zid
                zones_created += 1
        else:
            zones_skipped += len(src_zones)
        if copy_rules:
            for r in src_rules:
                if r["zone_id"] is not None:
                    src_zname = next((z["name"] for z in src_zones
                                      if z["id"] == r["zone_id"]), None)
                    zid = tgt_zone_names.get((src_zname + suffix) if src_zname else None)
                    if zid is None:
                        rules_skipped += 1
                        warnings.append(
                            f"目标 #{tid}：规则 {r['behavior']} 的源区域「{src_zname}」"
                            "在目标上不存在（未复制区域或同名缺失）——已跳过，不指向错误区域")
                        continue
                else:
                    zid = None  # 全画幅规则：目标同样 zone_id=None
                dup = any(x["behavior"] == r["behavior"] and x["zone_id"] == zid
                          and x["time_start"] == r["time_start"]
                          and x["time_end"] == r["time_end"] for x in ctx.store().rules(tid))
                if dup:
                    rules_skipped += 1
                    continue
                ctx.store().add_rule({"camera_id": tid, "zone_id": zid, "behavior": r["behavior"],
                                      "level": r["level"], "cooldown_s": r["cooldown_s"],
                                      "time_start": r["time_start"], "time_end": r["time_end"],
                                      "threshold": r["threshold"], "enabled": bool(r["enabled"])})
                rules_created += 1
        else:
            rules_skipped += len(src_rules)
        targets_out.append({"camera_id": tid, "zone_map": zone_map,
                            "zones_skipped": zones_skipped - z0,
                            "rules_skipped": rules_skipped - r0})
    return {"zones_created": zones_created, "zones_skipped": zones_skipped,
            "rules_created": rules_created, "rules_skipped": rules_skipped,
            "targets": targets_out, "warnings": warnings}


@router.get("/cameras/{cid}/status", dependencies=[Depends(ctx.read_guard)])
def camera_status(cid: int):
    """单路运行指标（任务65）：帧龄/最近错误（已脱敏）/重连次数/累计帧数。"""
    cam = ctx.store().camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    w = ctx.manager().workers.get(cid) if ctx.manager() else None
    out = {"id": cid, "name": cam["name"], "type": cam["type"], "status": cam["status"],
           "fps": None, "last_frame_ts": None, "frame_age_s": None, "last_error": None,
           "last_error_ts": None, "reconnect_count": 0, "opened_at": None, "frames_total": 0,
           "kind": "folder" if cam["type"] == "folder" else "video",
           # T-336：迁移期观测——本路分析器实现（python|rust），切换/回退以设置页为准
           "analyzer": cam.get("analyzer") or settings.analyzer_default}
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
        if hasattr(w, "folder_status"):
            out.update(w.folder_status())
    if ctx.manager() is not None:
        out.update(ctx.manager().supervisor_info(cid))  # 任务114：restart_count/守护状态
    return out
