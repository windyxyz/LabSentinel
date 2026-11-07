"""T-505 第三步（纯搬移）：zones / rules / rule-templates / calendar 路由。

从 api.py 原样搬出，行为不变；共享 Store 经 routers/_context.py 惰性代理。
"""
import json
import re

from fastapi import APIRouter, Depends, HTTPException

from . import _context as ctx
from ..models import CalendarBody, RuleCreate, TemplateApplyBody, TemplateBody, ZoneCreate

router = APIRouter()


# ---- zones ----
@router.get("/zones", dependencies=[Depends(ctx.read_guard)])
def zones(camera_id: int | None = None):
    return ctx.store().zones(camera_id)


@router.post("/zones", dependencies=[Depends(ctx.admin_guard)])
def add_zone(body: ZoneCreate):
    if ctx.store().camera(body.camera_id) is None:
        raise HTTPException(404, "camera not found")
    # 2026-10-30 审阅：入库前校验——坏 polygon 原先会静默入库，worker 每帧 parse 抛错，
    # 摄像头显示 running 却零事件。parse_polygon 的 ValueError 也在此一并转 400。
    if len(body.polygon) < 3:
        raise HTTPException(400, "安全区至少需要 3 个顶点")
    if any(len(pt) != 2 for pt in body.polygon):
        raise HTTPException(400, "顶点格式应为 [x, y]")
    try:
        zid = ctx.store().add_zone(body.camera_id, body.name, body.polygon)
    except ValueError as e:
        raise HTTPException(400, str(e))
    return {"id": zid}


@router.delete("/zones/{zid}", dependencies=[Depends(ctx.admin_guard)])
def delete_zone(zid: int):
    ctx.store().delete_zone(zid)
    return {"ok": True}


# ---- rules ----
@router.get("/rules", dependencies=[Depends(ctx.read_guard)])
def rules(camera_id: int | None = None):
    return ctx.store().rules(camera_id)


@router.post("/rules", dependencies=[Depends(ctx.admin_guard)])
def add_rule(body: RuleCreate):
    if body.behavior not in ("enter_no_ppe", "enter", "exit", "occupancy_over"):
        raise HTTPException(400, "unknown behavior")
    days = {d.strip() for d in body.days_of_week.split(",") if d.strip()}  # 任务112
    if not days or not days <= {str(i) for i in range(1, 8)}:
        raise HTTPException(400, "days_of_week 必须是 1-7 的逗号集合（ISO，1=周一；空=永不生效）")
    if ctx.store().camera(body.camera_id) is None:  # 2026-10-30 审阅：指向不存在摄像头的规则静默失活
        raise HTTPException(404, "camera not found")
    if body.zone_id is not None:
        zone = ctx.store().zone(body.zone_id)
        if zone is None:
            raise HTTPException(404, "zone not found")
        if zone["camera_id"] != body.camera_id:  # 105 补丁 P1-1：跨摄像头规则静默失活，入库即拒
            raise HTTPException(400, f"区域 {body.zone_id} 属于摄像头 #{zone['camera_id']}，"
                                     f"不能挂到摄像头 #{body.camera_id}")
    rid = ctx.store().add_rule(body.model_dump())
    return {"id": rid}


@router.delete("/rules/{rid}", dependencies=[Depends(ctx.admin_guard)])
def delete_rule(rid: int):
    ctx.store().delete_rule(rid)
    return {"ok": True}


# ---- 规则模板库（任务116：只存规则参数，不绑摄像头/区域）----
_TIME_RE = re.compile(r"^([01]\d|2[0-3]):[0-5]\d$")


def _validate_template_rules(rules) -> list[dict]:
    """模板规则逐条校验（任务116）；通过后返回清洗后的列表。"""
    if not isinstance(rules, list) or not rules or len(rules) > 50:
        raise HTTPException(400, "rules 必须为 1-50 条的非空数组")
    out = []
    for r in rules:
        try:
            body = RuleCreate.model_validate({
                "camera_id": 0,  # 模板不绑摄像头，占位过校验
                "behavior": r.get("behavior"), "level": r.get("level", 2),
                "cooldown_s": r.get("cooldown_s", 60),
                "time_start": r.get("time_start", "00:00"),
                "time_end": r.get("time_end", "23:59"),
                "threshold": r.get("threshold", 0),
                "days_of_week": r.get("days_of_week", "1,2,3,4,5,6,7"),
                "holiday_only": bool(r.get("holiday_only")),
            })
        except Exception as e:  # noqa: BLE001
            raise HTTPException(400, f"规则字段非法: {e}")
        if body.behavior not in ("enter_no_ppe", "enter", "exit", "occupancy_over"):
            raise HTTPException(400, f"未知 behavior: {body.behavior}")
        for k in ("time_start", "time_end"):
            if not _TIME_RE.match(getattr(body, k)):
                raise HTTPException(400, f"{k} 必须是 HH:MM")
        days = {d.strip() for d in body.days_of_week.split(",") if d.strip()}
        if not days or not days <= {str(i) for i in range(1, 8)}:
            raise HTTPException(400, "days_of_week 必须是 1-7 的逗号集合")
        out.append({k: getattr(body, k) for k in
                    ("behavior", "level", "cooldown_s", "time_start", "time_end",
                     "threshold", "days_of_week", "holiday_only")})
    return out


@router.get("/rule-templates", dependencies=[Depends(ctx.read_guard)])
def list_rule_templates():
    items = []
    for x in ctx.store().rule_templates():
        try:
            n = len(json.loads(x["config_json"]))
        except ValueError:
            n = 0
        items.append({"id": x["id"], "name": x["name"], "rules_count": n,
                      "updated_ts": x["updated_ts"]})
    return {"templates": items}


@router.post("/rule-templates", dependencies=[Depends(ctx.admin_guard)])
def create_rule_template(body: TemplateBody):
    name = body.name.strip()
    rules = _validate_template_rules(body.rules)
    tid = ctx.store().add_rule_template(name, json.dumps(rules, ensure_ascii=False))
    if tid is None:
        raise HTTPException(409, f"模板名已存在: {name}")
    return {"id": tid, "name": name, "rules": rules}


@router.put("/rule-templates/{tid}", dependencies=[Depends(ctx.admin_guard)])
def update_rule_template(tid: int, body: TemplateBody):
    name = body.name.strip()
    rules = _validate_template_rules(body.rules)
    rc = ctx.store().update_rule_template(tid, name, json.dumps(rules, ensure_ascii=False))
    if rc == 0:
        raise HTTPException(404, "template not found")
    if rc == -1:
        raise HTTPException(409, f"模板名已存在: {name}")
    return {"id": tid, "name": name, "rules": rules}


@router.delete("/rule-templates/{tid}", dependencies=[Depends(ctx.admin_guard)])
def delete_rule_template(tid: int):
    if not ctx.store().delete_rule_template(tid):
        raise HTTPException(404, "template not found")
    return {"ok": True}


@router.post("/rule-templates/{tid}/apply", dependencies=[Depends(ctx.admin_guard)])
def apply_rule_template(tid: int, body: TemplateApplyBody):
    """套用模板到多路：zone_id 须属该摄像头（或空=全画幅全局规则）；不自动启动。"""
    tpl = ctx.store().get_rule_template(tid)
    if tpl is None:
        raise HTTPException(404, "template not found")
    rules = json.loads(tpl["config_json"])
    cam_ids = body.camera_ids
    zone_id = body.zone_id
    created, skipped = 0, 0
    for cid in cam_ids:
        cam = ctx.store().camera(int(cid))
        if cam is None:
            raise HTTPException(404, f"camera {cid} not found")
        if zone_id is not None:
            z = ctx.store().zone(int(zone_id))
            if z is None or z["camera_id"] != cam["id"]:
                raise HTTPException(400, f"区域 {zone_id} 不属于摄像头 #{cam['id']}")
        for r in rules:
            dup = any(x["behavior"] == r["behavior"]
                      and (x["zone_id"] or None) == zone_id
                      and x["time_start"] == r["time_start"]
                      and x["time_end"] == r["time_end"]
                      for x in ctx.store().rules(cam["id"]))
            if dup:
                skipped += 1
                continue
            ctx.store().add_rule({**r, "camera_id": cam["id"], "zone_id": zone_id, "enabled": True})
            created += 1
    return {"created": created, "skipped": skipped,
            "cameras": len(cam_ids), "rules_per_camera": len(rules)}


# ---- 日历例外（任务112：holiday=该日按假期 / workday=该日按工作日；覆盖周末判断）----
@router.get("/calendar", dependencies=[Depends(ctx.read_guard)])
def list_calendar():
    return {"exceptions": ctx.store().calendar_exceptions()}


@router.post("/calendar", dependencies=[Depends(ctx.admin_guard)])
def set_calendar(body: CalendarBody):
    d = body.date.strip()
    mode = body.mode
    note = body.note
    try:
        from datetime import date as _date
        _date.fromisoformat(d)
    except ValueError as e:
        raise HTTPException(400, f"date 必须是 YYYY-MM-DD: {e}")
    if mode not in ("holiday", "workday"):
        raise HTTPException(400, "mode 必须是 holiday|workday")
    replaced = ctx.store().set_calendar_exception(d, mode, note)
    return {"date": d, "mode": mode, "note": note, "replaced": replaced}  # 覆盖语义：同日期 upsert


@router.delete("/calendar/{d}", dependencies=[Depends(ctx.admin_guard)])
def del_calendar(d: str):
    if not ctx.store().delete_calendar_exception(d):
        raise HTTPException(404, "calendar exception not found")
    return {"ok": True}
