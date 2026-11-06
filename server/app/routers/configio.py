"""T-505 第四步（纯搬移）：配置导入导出（任务79）。

从 api.py 原样搬出，行为不变；共享对象经 routers/_context.py 惰性代理。
"""
import re
from datetime import datetime
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException
from fastapi.responses import JSONResponse

from . import _context as ctx

router = APIRouter()

_CONFIG_FORMAT = "labsentinel-config"
_RTSP_CRED = re.compile(r"(//)([^/@\s:]+):([^@\s]+)@")


@router.get("/config/export", dependencies=[Depends(ctx.admin_guard)])
def config_export(with_secrets: int = 0):
    """配置级导出（不含事件/截图/人脸）。默认把 RTSP 凭证脱敏为 ***:***@；
    with_secrets=1 返回真实 source（require_admin 已要求令牌；响应 no-store）。"""
    store = ctx.store()
    cams = store.cameras()
    cname = {c["id"]: c["name"] for c in cams}
    zname = {z["id"]: z["name"] for z in store.zones()}
    cameras = []
    for c in cams:
        src = c["source"] or ""
        if not with_secrets:
            src = _RTSP_CRED.sub(r"\1***:***@", src)
        cameras.append({"name": c["name"], "source": src, "type": c["type"]})
    zones = [{"camera_name": cname.get(z["camera_id"], f"#{z['camera_id']}"),
              "name": z["name"], "polygon": z["polygon"]} for z in store.zones()]
    rules = [{"camera_name": cname.get(r["camera_id"], f"#{r['camera_id']}"),
              "zone_name": zname.get(r["zone_id"]) if r["zone_id"] else None,
              "behavior": r["behavior"], "level": r["level"], "cooldown_s": r["cooldown_s"],
              "time_start": r["time_start"], "time_end": r["time_end"],
              "threshold": r["threshold"], "enabled": bool(r["enabled"])} for r in store.rules()]
    payload = {"format": _CONFIG_FORMAT, "version": 1,
               "schema_version": store.schema_version(),
               "exported_at": datetime.now().isoformat(timespec="seconds"),
               "cameras": cameras, "zones": zones, "rules": rules}
    return JSONResponse(payload, headers={"Cache-Control": "no-store"})


def _validate_config_import(data: dict):
    """全量校验（先校验后写库）：返回 (cameras, zones, rules)。失败抛 400，不产生部分写入。"""
    if not isinstance(data, dict) or data.get("format") != _CONFIG_FORMAT:
        raise HTTPException(400, f"format 必须为 {_CONFIG_FORMAT}")
    if int(data.get("version", 0)) != 1:
        raise HTTPException(400, "version 必须为 1")
    errs: list[str] = []
    cams, zones_in, rules_in = [], data.get("zones") or [], data.get("rules") or []
    # 任务134：规模上限在填充循环前，直接量原始输入（cams 此刻还是空列表）
    if len(data.get("cameras") or []) > 200 or len(zones_in) > 1000 or len(rules_in) > 2000:
        raise HTTPException(400, "导入规模超限（摄像头≤200/区域≤1000/规则≤2000）")
    seen = set()
    for c in (data.get("cameras") or []):
        name = (c.get("name") or "").strip()
        if not name or name in seen:
            errs.append(f"摄像头 name 空/重名: {name!r}")
            continue
        if c.get("type") not in ("rtsp", "file", "usb", "folder"):
            errs.append(f"摄像头 {name} type 非法: {c.get('type')}")
            continue
        if not (c.get("source") or "").strip():
            errs.append(f"摄像头 {name} source 为空")
            continue
        # T-103：导入路径同样做 source 白名单校验（file 允许跨机器缺失路径）
        from ..sources import SourceError, validate_source
        try:
            src = validate_source(c["type"], c["source"], allow_missing=True)
        except SourceError as e:
            errs.append(f"摄像头 {name} source 非法: {e}")
            continue
        seen.add(name)
        cams.append({"name": name, "source": src, "type": c["type"]})
    for z in zones_in:
        cname, zname = z.get("camera_name"), (z.get("name") or "").strip()
        poly = z.get("polygon")
        if cname not in seen:
            errs.append(f"区域 {zname!r} 引用未知摄像头: {cname!r}")
        elif (not isinstance(poly, list) or len(poly) < 3
              or any(not isinstance(pt, list) or len(pt) != 2
                     or not all(isinstance(v, int) for v in pt) for pt in poly)):
            errs.append(f"区域 {zname!r} polygon 至少 3 个整数 [x,y]")
        elif any(not (0 <= pt[0] <= 1280) or not (0 <= pt[1] <= 720) for pt in poly):
            errs.append(f"区域 {zname!r} 坐标越界（0≤x≤1280、0≤y≤720）")
        else:
            zones_in[zones_in.index(z)] = {"camera_name": cname, "name": zname, "polygon": poly}
    for r in rules_in:
        cname = r.get("camera_name")
        if cname not in seen:
            errs.append(f"规则引用未知摄像头: {cname!r}")
            continue
        if r.get("behavior") not in ("enter_no_ppe", "enter", "exit", "occupancy_over"):
            errs.append(f"规则 behavior 非法: {r.get('behavior')}")
            continue
        if not (1 <= int(r.get("level", 2)) <= 3):
            errs.append(f"规则 level 非法: {r.get('level')}")
            continue
        if r.get("behavior") != "occupancy_over" and not r.get("zone_name"):
            errs.append(f"规则（{cname}/{r.get('behavior')}）缺 zone_name")
    errs = [e for e in errs if e]
    if errs:
        raise HTTPException(400, "导入校验失败（未写入任何数据）: " + "；".join(errs[:8]))
    return cams, zones_in, rules_in


@router.post("/config/import", dependencies=[Depends(ctx.admin_guard)])
def config_import(body: dict):
    """配置级导入（任务79）：merge 按名称去重；replace 需 confirm_replace=true
    （事件保留，但其 camera_id 将失去对应摄像头）。不自动启动摄像头。"""
    mode = body.get("mode", "merge")
    if mode not in ("merge", "replace"):
        raise HTTPException(400, "mode 必须为 merge|replace")
    cams, zones, rules = _validate_config_import(body.get("data"))
    if mode == "replace":
        if not body.get("confirm_replace"):
            raise HTTPException(400, "replace 模式必须 confirm_replace=true"
                                     "（将清空现有摄像头/区域/规则；事件保留但失去对应摄像头）")
        if ctx.manager() is not None:
            ctx.manager().stop_all()  # 引用现有摄像头的 worker 先停
        ctx.store().clear_config()
    created = {"cameras": 0, "zones": 0, "rules": 0}
    skipped = {"cameras": [], "zones": [], "rules": []}
    warns: list[str] = []
    cam_id_by_name = {c["name"]: c["id"] for c in ctx.store().cameras()}
    existing_zones = {(z["camera_id"], z["name"]) for z in ctx.store().zones()}
    for c in cams:
        if c["name"] in cam_id_by_name:
            skipped["cameras"].append(c["name"])
            continue
        cam_id_by_name[c["name"]] = ctx.store().add_camera(c["name"], c["source"], c["type"])
        created["cameras"] += 1
        if c["type"] == "folder" and not Path(str(c["source"])).is_dir():
            warns.append(f"图片文件夹不存在：{c['name']} → {c['source']}（启动后为 error，"
                         "路径恢复后会自动重试）")
    for z in zones:
        cid = cam_id_by_name[z["camera_name"]]
        if (cid, z["name"]) in existing_zones:
            skipped["zones"].append(z["name"])
            continue
        zid = ctx.store().add_zone(cid, z["name"], z["polygon"])
        existing_zones.add((cid, z["name"]))
        created["zones"] += 1
    # 重建 zone_name → (camera_name, zid) 映射（含 merge 已有区域）
    zkey_to_zid = {}
    zname_by_id = {z["id"]: z["name"] for z in ctx.store().zones()}
    cam_name_by_id = {c["id"]: c["name"] for c in ctx.store().cameras()}
    for z in ctx.store().zones():
        zkey_to_zid[(cam_name_by_id.get(z["camera_id"]), z["name"])] = z["id"]
    existing_rules = {(cam_name_by_id.get(r["camera_id"]), r["behavior"],
                       zname_by_id.get(r["zone_id"]), r["time_start"], r["time_end"])
                      for r in ctx.store().rules()}
    for r in rules:
        cid = cam_id_by_name[r["camera_name"]]
        zid = zkey_to_zid.get((r["camera_name"], r["zone_name"])) if r["zone_name"] else None
        key = (r["camera_name"], r["behavior"], r["zone_name"], r["time_start"], r["time_end"])
        if r["zone_name"] and zid is None:
            skipped["rules"].append(f"{r['camera_name']}/{r['behavior']}（区域未创建）")
            continue
        if key in existing_rules:
            skipped["rules"].append(f"{r['camera_name']}/{r['behavior']}")
            continue
        ctx.store().add_rule({"camera_id": cid, "zone_id": zid, "behavior": r["behavior"],
                              "level": r["level"], "cooldown_s": r["cooldown_s"],
                              "time_start": r["time_start"], "time_end": r["time_end"],
                              "threshold": r["threshold"], "enabled": bool(r["enabled"])})
        existing_rules.add(key)
        created["rules"] += 1
    if any(skipped.values()):
        warns.append(f"跳过已存在条目：cameras={skipped['cameras']} zones={skipped['zones']} "
                     f"rules={skipped['rules']}")
    if mode == "replace":
        warns.append("事件保留，但事件中的 camera_id 已失去对应摄像头（重新导入同名摄像头可恢复关联）")
    return {"created": created, "skipped": skipped, "warnings": warns}


@router.get("/cameras/{cid}/snapshot", dependencies=[Depends(ctx.read_guard)])
def camera_snapshot(cid: int):
    frame = ctx.manager().latest_frame(cid)
    if frame is None:
        raise HTTPException(404, "no frame yet")
    import cv2
    ok, buf = cv2.imencode(".jpg", frame)
    if not ok:
        raise HTTPException(500, "encode failed")
    from fastapi.responses import Response
    return Response(content=buf.tobytes(), media_type="image/jpeg")
