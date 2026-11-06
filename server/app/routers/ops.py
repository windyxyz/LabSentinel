"""T-505 第二步（纯搬移）：face / alerts / maintenance / diagnostics 路由。

从 api.py 原样搬出，行为不变；共享对象经 routers/_context.py 惰性代理。
"""
import os
import shutil
from datetime import datetime, timedelta

from fastapi import APIRouter, Depends, File, Form, HTTPException, UploadFile
from fastapi.responses import Response  # noqa: F401  保持与 api.py 相同的响应类型可用

from . import _context as ctx
from ..config import settings

router = APIRouter()


def _face_guard():
    if not settings.face_enabled:
        raise HTTPException(403, "人脸比对功能未开启（LS_FACE_ENABLED=false）。开启方式与合规要求见 README。")
    if not ctx.face_client().configured:
        raise HTTPException(503, "人脸服务未配置（缺少 LS_COMPREFACE_API_KEY）")


# ---- face（合规敏感：默认关闭，见 app/face.py）----
@router.get("/face/status")
def face_status():
    """人脸功能状态（只读展示用；开关只能改配置重启，不提供界面开关——防误开）。"""
    return {"enabled": settings.face_enabled, "configured": ctx.face_client().configured,
            "threshold": settings.face_threshold}


@router.post("/face/persons", dependencies=[Depends(ctx.admin_guard)])
async def face_register(subject: str = Form(...), consent: bool = Form(...),
                        photo: UploadFile = File(...)):
    """注册人员：照片转存 CompreFace，同意记录落本地库。功能关闭时 403。"""
    _face_guard()
    if not consent:
        raise HTTPException(400, "注册人员必须 consent=true（《人脸识别技术应用安全管理办法》要求留痕）")
    max_bytes = 10 * 1024 * 1024  # 任务134：单张照片上限 10MB（防内存滥用）
    size = getattr(photo, "size", None)
    if isinstance(size, int) and size > max_bytes:  # 已声明大小：先拒，避免整包读入内存
        raise HTTPException(413, "照片超过 10MB 上限")
    data = await photo.read(max_bytes + 1)  # 最多读 10MB+1，超限即拒
    if not data:
        raise HTTPException(400, "照片为空")
    if len(data) > max_bytes:
        raise HTTPException(413, "照片超过 10MB 上限")
    try:
        ctx.face_client().add_person(subject, data)
    except Exception as e:  # noqa: BLE001
        raise HTTPException(502, f"CompreFace 注册失败: {e}")
    ctx.store().add_face_consent(subject)
    return {"ok": True, "subject": subject, "consent_ts": ctx.store().face_consent_ts(subject)}


@router.get("/face/persons", dependencies=[Depends(ctx.read_guard)])
def face_persons():
    _face_guard()
    consents = {r["subject"]: r["ts"] for r in ctx.store().face_consents()}
    try:
        persons = ctx.face_client().list_persons()
    except Exception as e:  # noqa: BLE001
        raise HTTPException(502, f"CompreFace 人员列表获取失败: {e}")
    return [{"subject": s, "consent_ts": consents.get(s)} for s in persons]


@router.delete("/face/persons/all", dependencies=[Depends(ctx.admin_guard)])
def face_clear_all():
    """一键清除：人员库 + 同意记录（合规要求）。"""
    _face_guard()
    errors = []
    for s in ctx.face_client().list_persons():
        try:
            ctx.face_client().delete_person(s)
        except Exception as e:  # noqa: BLE001
            errors.append(f"{s}: {e}")
    ctx.store().clear_face_consents()
    return {"ok": not errors, "errors": errors}


@router.delete("/face/persons/{subject}", dependencies=[Depends(ctx.admin_guard)])
def face_delete(subject: str):
    _face_guard()
    try:
        ctx.face_client().delete_person(subject)
    except Exception as e:  # noqa: BLE001
        raise HTTPException(502, f"CompreFace 删除失败: {e}")
    return {"ok": True}


# ---- alert deliveries（任务13：推送可靠性）----
@router.get("/alerts/status", dependencies=[Depends(ctx.read_guard)])
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
    by_ch = ctx.dispatcher().store.delivery_counts_by_channel() if ctx.dispatcher().store else {}
    channels = []
    for ch in ctx.dispatcher().channels:
        name = getattr(ch, "name", type(ch).__name__)
        channels.append({"name": name,
                         "configured": configured_map.get(name, False),
                         "deliveries": by_ch.get(name, {"ok": 0, "failed": 0, "pending": 0})})
    from ..alerts import alert_filters
    filters = alert_filters()
    channels = [{**c, "filters": filters.get(c["name"], {})} for c in channels]
    return {"channels": channels, "test_supported": True}


# ---- 网页端告警配置（任务140；仅管理员；密钥不回显）----
@router.get("/alerts/config", dependencies=[Depends(ctx.admin_guard)])
def alerts_config_get():
    return ctx.alert_config().status(ctx.store())


@router.put("/alerts/config", dependencies=[Depends(ctx.admin_guard)])
def alerts_config_put(body: dict):
    try:
        return ctx.alert_config().save(ctx.store(), body)
    except ctx.alert_config().AlertConfigError as e:
        raise HTTPException(400, str(e))


@router.post("/alerts/config/reset", dependencies=[Depends(ctx.admin_guard)])
def alerts_config_reset(channel: str | None = None):
    """清除网页保存的覆盖，回退到启动时 .env 值（channel 为空=全部）。"""
    try:
        return ctx.alert_config().reset(ctx.store(), channel)
    except ctx.alert_config().AlertConfigError as e:
        raise HTTPException(400, str(e))


@router.get("/alert-deliveries", dependencies=[Depends(ctx.read_guard)])
def alert_deliveries(status: str | None = None, limit: int = 50):
    if not ctx.dispatcher().store:
        raise HTTPException(503, "store 未绑定（服务未完成启动）")
    return ctx.dispatcher().store.deliveries(status=status, limit=max(1, min(limit, 200)))


@router.post("/alert-deliveries/{did}/retry", dependencies=[Depends(ctx.admin_guard)])
def alert_delivery_retry(did: int):
    """死信手动重发：按原渠道、原事件文案重投（重新走三次退避）。"""
    d = ctx.dispatcher()
    if not d.store:
        raise HTTPException(503, "store 未绑定")
    rec = d.store.delivery(did)
    if rec is None:
        raise HTTPException(404, "delivery not found")
    ch = next((c for c in d.channels if getattr(c, "name", "") == rec["channel"]), None)
    if ch is None:
        raise HTTPException(400, f"未知渠道: {rec['channel']}")
    if rec["event_id"]:
        row = ctx.store().event_by_id(rec["event_id"])  # 2026-10-30 审阅：按 id 单查（原全表拉取）
        if row is None:
            raise HTTPException(404, "原事件已不存在")
        text = ctx.render_alert(row)
        level = row["level"]
    else:  # event_id=0 为测试消息
        level, text = 1, "【重发测试】LabSentinel 渠道自检消息。"
    d.store.update_delivery(did, "pending", rec["attempts"], None)
    d._pool.submit(d._deliver_with_retry, ch, rec["event_id"], level, text)
    return {"ok": True, "retrying": did}


@router.post("/alert-test", dependencies=[Depends(ctx.admin_guard)])
def alert_test(channel: str | None = None):
    """渠道健康自检：默认测全部已配置渠道；channel 指定时只测该渠道。"""
    return ctx.dispatcher().send_test(channel=channel)


# ---- maintenance（任务14：数据保留/清理）----
@router.post("/maintenance/cleanup", dependencies=[Depends(ctx.admin_guard)])
def maintenance_cleanup():
    """手动触发一轮清理（幂等）。返回 {deleted_events, deleted_files, freed_mb, disk_free_gb}。"""
    from .. import retention
    return retention.run_cleanup(ctx.store())


@router.get("/diagnostics/summary", dependencies=[Depends(ctx.read_guard)])
def diagnostics_summary():
    """诊断聚合（任务61）：只读、可公开的聚合信息；错误取自进程内 RingHandler。"""
    from .. import diagnostics
    return diagnostics.summary(ctx.store())


@router.get("/maintenance/preview", dependencies=[Depends(ctx.read_guard)])
def maintenance_preview():
    """清理预演（任务68）：只读，返回将删除的事件数/字节与释放预估，绝不删除。"""
    from .. import retention
    return retention.preview_cleanup(ctx.store())


@router.get("/maintenance/status", dependencies=[Depends(ctx.read_guard)])
def maintenance_status():
    """维护状态（任务14/68）：清理结果 + 磁盘水位/数据体量/增速预估（估算值已标注）。"""
    from .. import retention

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
        with ctx.store()._conn() as c:
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
