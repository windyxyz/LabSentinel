"""REST API 路由。"""
import csv
import io
import json
import mimetypes
import os
import re
import secrets
import shutil
import sqlite3
import time
from datetime import datetime, timedelta
from pathlib import Path

from fastapi import (
    APIRouter,
    Depends,
    File,
    Form,
    HTTPException,
    Request,
    UploadFile,
    WebSocket,
    WebSocketDisconnect,
)
from fastapi.responses import FileResponse, JSONResponse, Response, StreamingResponse

from . import (
    alert_config,
    discovery,
    genai,
    go2rtc,
    keepawake,
    mqtt_bridge,
    update_check,
)
from . import plugins as plugins_mod
from .alerts import dispatcher, render_alert
from .config import settings
from .constants import TYPE_TEXT

# B-28：api 层不再导入 get_engine——/api/system 只读 engine._engine/PRELOAD，模型缺失不炸
from .face import FaceClient
from .folders import image_exts, thumbnail_bytes
from .models import CameraCreate, RuleCreate, ZoneCreate
from .pipeline import CameraManager
from .stats import daily as stats_daily_query
from .stats import hourly as stats_hourly_query
from .stats import quality as stats_quality_query
from .stats import summary as stats_summary_query
from .stats import timeline as stats_timeline_query
from .stats import weekly as stats_weekly_query
from .store import EVENT_STATUSES, Store, ro_uri

router = APIRouter()  # T-621：前缀由 main 挂载（/api 别名 + /api/v1 正式）

store: Store | None = None
manager: CameraManager | None = None
face_client = FaceClient()  # 依据 settings 配置；未配置时 configured=False

# ---- 鉴权（任务16）：LS_ADMIN_TOKEN 为空 = 不启用；写操作需 X-Admin-Token ----
WS_AUTH_CODE = 4401


def _token_ok(provided: str | None) -> bool:
    """管理令牌校验（写操作专用，任务88 语义收紧）。"""
    if not settings.admin_token:
        return True  # 未启用鉴权
    if not provided:
        return False
    import hmac
    return hmac.compare_digest(provided, settings.admin_token)


def _read_token_ok(provided: str | None) -> bool:
    """读令牌校验（任务88）：admin_token 或 read_token 均可（hmac.compare_digest）。"""
    if not settings.admin_token:
        return True  # 未启用鉴权
    if not provided:
        return False
    import hmac
    if hmac.compare_digest(provided, settings.admin_token):
        return True
    return bool(settings.read_token) and hmac.compare_digest(provided, settings.read_token)


def _warn_same_token():
    """任务88：read_token 与 admin_token 相同 → 弱隔离 WARNING（允许运行）。"""
    if settings.admin_token and settings.read_token and \
            settings.admin_token == settings.read_token:
        print("[auth] ⚠ LS_READ_TOKEN 与 LS_ADMIN_TOKEN 相同——只读令牌形同虚设（弱隔离）")


_warn_same_token()


def require_admin(request: Request):
    """写操作守卫：只接受管理令牌（任务88：只读令牌写永拒）。T-113：结果写入 state.actor。"""
    if not _token_ok(request.headers.get("X-Admin-Token")):
        request.state.actor = "rejected"
        if settings.admin_token and settings.read_token and \
                request.headers.get("X-Admin-Token") == settings.read_token:
            raise HTTPException(401, "当前令牌无写权限（只读令牌）——请使用管理令牌")
        raise HTTPException(401, "需要管理令牌（X-Admin-Token）")
    request.state.actor = "admin" if settings.admin_token else "anonymous(auth-off)"


def require_triage(request: Request):
    """T-620/Q-11：事件处置守卫——管理令牌或设备令牌（可处置不可改配置）。"""
    if not settings.admin_token:
        request.state.actor = "anonymous(auth-off)"
        return
    provided = request.headers.get("X-Admin-Token")
    if _token_ok(provided):
        request.state.actor = "admin"
        return
    from . import devices
    dev = devices.verify(store, provided or "") if store is not None else None
    if dev is None:
        request.state.actor = "rejected"
        raise HTTPException(401, "需要管理令牌或已配对设备令牌")
    request.state.actor = f"device:{dev['name']}"


def _media_sig_ok(request: Request) -> bool:
    """T-108：短期签名媒体 URL（HMAC over path|exp），有效期 LS_MEDIA_SIGN_TTL_S。
    密钥 = admin_token 派生（未设令牌时用 .env 无关的进程内随机密钥）。"""
    import hashlib
    import hmac as _hmac
    exp = request.query_params.get("exp", "")
    sig = request.query_params.get("sig", "")
    try:
        if int(exp) < int(time.time()):
            return False
    except (ValueError, TypeError):
        return False
    secret = (settings.admin_token or _MEDIA_FALLBACK_KEY).encode()
    base_path = request.url.path
    want = _hmac.new(secret, f"{base_path}|{exp}".encode(), hashlib.sha256).hexdigest()
    return _hmac.compare_digest(sig, want)


_MEDIA_FALLBACK_KEY = secrets.token_urlsafe(32)


def require_read(request: Request):
    """读操作守卫：仅 LS_AUTH_READ_PROTECTED=true 时生效（T-113 写 state.actor）。

    105 回归补丁 P0-2：`<img>/<video>/<a download>` 裸 URL 不带请求头——
    T-108 起优先用签名 URL（`?exp=&sig=` HMAC，不落令牌明文）；`?token=` 保留兼容
    （LS_ALLOW_URL_TOKEN=0 关闭；弃用期两个版本后移除）。
    """
    if not settings.auth_read_protected:
        request.state.actor = "anonymous(auth-off)"
        return
    if _media_sig_ok(request):
        request.state.actor = "read(signed)"
        return
    provided = request.headers.get("X-Admin-Token")
    if settings.allow_url_token:
        provided = provided or request.query_params.get("token")
    if not _read_token_ok(provided):
        request.state.actor = "rejected"
        raise HTTPException(401, "需要管理令牌或签名媒体 URL")
    request.state.actor = "read"


@router.get("/media/sign", dependencies=[Depends(require_read)])
def sign_media_url(path: str):
    """T-108：给媒体路径签发 5 分钟短期 URL 参数（前端批量签名，令牌不进 URL/日志）。"""
    import hashlib
    import hmac as _hmac
    if not path.startswith("/api/"):
        raise HTTPException(400, "只能签名 /api/ 媒体路径")
    if "?" in path or "#" in path or " " in path:
        raise HTTPException(400, "path 只能含路径（查询串自行拼接在签名后）")
    exp = str(int(time.time()) + settings.media_sign_ttl_s)
    secret = (settings.admin_token or _MEDIA_FALLBACK_KEY).encode()
    sig = _hmac.new(secret, f"{path}|{exp}".encode(), hashlib.sha256).hexdigest()
    return {"exp": exp, "sig": sig}


def init(s: Store, m: CameraManager):
    global store, manager
    store, manager = s, m


AI_VERDICTS = ("likely_true", "likely_false", "uncertain", "none")


def _check_ai_verdict(v: str | None) -> None:
    """任务132：ai_verdict 过滤参数校验（None=不过滤；非法 400）。"""
    if v is not None and v not in AI_VERDICTS:
        raise HTTPException(400, "ai_verdict 必须是 likely_true|likely_false|uncertain|none")


def _event_row(row: dict) -> dict:
    row = dict(row)
    row["has_snapshot"] = bool(row.get("snapshot"))
    row["has_video"] = bool(row.get("video"))
    row["status"] = row.get("status") or "new"  # 旧库迁移前列缺失时兜底
    raw_ai = row.get("ai_review")  # 任务124：VLM 复核结果（JSON 字符串 → 对象+verdict 快捷字段）
    if raw_ai:
        try:
            row["ai_review"] = json.loads(raw_ai)
        except ValueError:
            row["ai_review"] = None
    else:
        row["ai_review"] = None
    row["ai_verdict"] = (row["ai_review"] or {}).get("verdict")
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
        timeout_s = max(0.0, int(settings.sqlite_busy_timeout_ms)) / 1000.0
        con = sqlite3.connect(ro_uri(store.path), uri=True, timeout=timeout_s)  # 任务76：URI 编码安全
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
def _config_issue_counts() -> dict:
    """任务95：启动校验 FAIL/WARN 计数（不泄露路径）。"""
    from .validate import fail_count, validate_settings, warn_count
    r = validate_settings()
    return {"fail": fail_count(r), "warn": warn_count(r)}


def _system_version() -> str:
    from .version import read_version
    return read_version()


def _engine_backend_peek() -> str:
    """B-28：只读后端名，绝不触发模型加载（模型缺失/损坏时 /api/system 也必须 200）。
    口径与 /api/model/status 一致；引擎未建时回退配置顺序首项。"""
    from . import engine as _eng_mod
    if _eng_mod._engine is not None:
        return _eng_mod._engine.backend
    from .engine import default_backend_order
    order = settings.backend_order or default_backend_order()  # T-603：空=平台默认
    return _eng_mod.PRELOAD.get("backend") or order.split(",")[0].strip()


@router.get("/system")
def system():
    today = store.today_event_counts()
    unhandled = sum(n for lv, n in today.items()) - store.count_events_today_handled()
    from . import engine as _eng_mod
    return {
        "version": _system_version(),
        "schema_version": store.schema_version(),  # 任务69
        "backend": _engine_backend_peek(),
        **(manager.status() if manager else {"cameras": []}),
        "today_counts": today,
        "unhandled_today": max(0, unhandled),  # 今日未处置（任务12）
        "alerts": dispatcher.store.delivery_counts() if dispatcher.store else {"pending": 0, "failed": 0, "ok": 0},
        "config_issues": {**_config_issue_counts(),
                          "source_issues": startup_source_issues() if store else []},
        "keep_awake": keepawake.status(),  # 任务123：防休眠状态
        "components": {
            "engine": dict(_eng_mod.PRELOAD),  # B-28：预热状态与错误如实上报
            "go2rtc": go2rtc.health(),         # T-101/T-202：回环鉴权网关健康
        },
    }


def startup_source_issues() -> list[str]:
    """T-103：启动迁移检查——已存摄像头中不合规 source 逐条报告（不自动删除/改写）。"""
    from .sources import startup_issues
    try:
        return startup_issues(list(store.cameras()) if store else [])
    except Exception as e:  # noqa: BLE001 迁移检查失败不拖垮主接口
        return [f"source 迁移检查异常: {type(e).__name__}"]


# ---- 服务控制（任务120：网页端停止）----
SHUTDOWN_DELAY_S = 0.8  # 留给响应送达客户端的窗口


def _do_shutdown() -> None:
    """退出前清启动器锁（便携布局锁在 data 上一级 logs/；源码模式该路径不存在，静默）。"""
    import os
    import time
    from pathlib import Path as _P
    time.sleep(SHUTDOWN_DELAY_S)
    try:
        lock = _P(str(settings.data_dir)).parent / "logs" / "launcher.lock"
        lock.unlink(missing_ok=True)
    except OSError:
        pass
    os._exit(0)  # 跳过 lifespan 收尾：与 taskkill /F 同级口径；WAL 崩溃安全


@router.post("/system/shutdown", dependencies=[Depends(require_admin)])
def shutdown_server():
    """网页端停止服务（任务120）：响应返回后 ~1s 进程退出。

    require_admin 口径与其它写接口一致（未设令牌=开放）。测试注入：monkeypatch _do_shutdown。
    """
    import threading
    threading.Thread(target=_do_shutdown, daemon=True, name="shutdown").start()
    return {"ok": True, "msg": "服务即将停止，可关闭本页面"}


# ---- 模型/配置状态（任务91，只读）----
@router.get("/model/status", dependencies=[Depends(require_read)])
def model_status():
    """模型与推理配置只读状态（不加载 ONNX、不触发推理）。"""
    from . import engine as _eng_mod
    backend = _eng_mod._engine.backend if _eng_mod._engine is not None         else settings.backend_order.split(",")[0].strip()

    def _file_info(p):
        from pathlib import Path as _P
        fp = _P(str(p))
        if not fp.exists():
            return {"path": str(p), "exists": False, "size_bytes": None, "mtime": None}
        return {"path": str(p), "exists": True,
                "size_bytes": fp.stat().st_size,
                "mtime": datetime.fromtimestamp(fp.stat().st_mtime).isoformat(timespec="seconds")}

    models = {"ppe": _file_info(settings.ppe_model or settings.model_fp32),
              "fp32": _file_info(settings.model_fp32)}
    if "vitisai" in settings.backend_order.split(","):  # T-204（B-07）：int8 条目应报 VitisAI 模型
        models["int8"] = _file_info(settings.model_int8)

    # classes 文件（与 PpeEngine 一致：模型同目录 <stem>.classes.txt）
    classes_file = None
    ppe_p = settings.ppe_model or settings.model_fp32
    if ppe_p:
        stem = Path(str(ppe_p)).with_suffix("")
        cand = stem.with_suffix(".classes.txt")
        if not cand.exists():
            cand = stem.parent / "yolo11n_ppe.classes.txt"
        if cand.exists():
            classes_file = str(cand)
    class_names, class_count = [], 0
    if classes_file and Path(classes_file).exists():
        names_raw = Path(classes_file).read_text(encoding="utf-8", errors="replace").splitlines()
        class_names = [n.strip() for n in names_raw[:50] if n.strip()]
        class_count = len([n for n in names_raw if n.strip()])

    return {"backend": backend, "imgsz": settings.imgsz, "conf": settings.conf_thres,
            "analysis_fps": settings.analysis_fps,
            "models": models,
            "classes": {"file": classes_file, "exists": classes_file is not None,
                        "count": class_count, "names": class_names},
            "ppe": {"class_names": settings.ppe_class_names,
                    "violation_class": settings.ppe_violation_class,
                    "conf": settings.ppe_conf, "wearing_conf": settings.ppe_wearing_conf},
            "face_enabled": settings.face_enabled}


@router.get("/config/summary", dependencies=[Depends(require_admin)])
def config_summary():
    """当前生效配置脱敏摘要（任务91）：TOKEN/PASSWORD/SECRET/KEY/PASS/CREDENTIAL 全 ***。"""
    from .redact import redact_line
    out = []
    for k, v in settings.model_dump().items():
        out.append(redact_line(f"LS_{k.upper()}", str(v)))
    return {"keys": out}


# ---- 自动备份（任务92）----
@router.get("/backups/status", dependencies=[Depends(require_read)])
def backups_status():
    from . import backups
    return backups.status()


@router.post("/backups/run", dependencies=[Depends(require_admin)])
def backups_run():
    from . import backups
    if not backups.trigger_async():  # T-204（B-09）：原子触发，并发第二次 409
        raise HTTPException(409, "备份正在进行中")
    return {"started": True}


# ---- 设备配对（T-620/Q-11）----
@router.post("/pairing", dependencies=[Depends(require_admin)])
def create_pairing():
    """生成一次性配对码（6 位数字，5 分钟有效，管理员操作）。"""
    from . import devices
    code = devices.new_pairing_code()
    return {"code": code, "expires_in_s": 300,
            "claim": "POST /api/pairing/claim {code, device_name, platform}"}


@router.post("/pairing/claim")
def claim_pairing(body: dict):
    """客户端用配对码换设备令牌（只返回一次；服务端只存 hash）。"""
    from . import devices
    code = str(body.get("code") or "").strip()
    name = str(body.get("device_name") or "device")[:64]
    platform = str(body.get("platform") or "")[:32]
    if store is None:
        raise HTTPException(503, "服务未就绪")
    token = devices.claim(store, code, name, platform)
    if token is None:
        raise HTTPException(400, "配对码无效或已过期")
    return {"token": token, "note": "令牌仅显示一次，请保存到安全存储"}


@router.get("/devices", dependencies=[Depends(require_admin)])
def list_devices():
    from . import devices
    return devices.list_devices(store)


@router.delete("/devices/{did}", dependencies=[Depends(require_admin)])
def revoke_device(did: int):
    from . import devices
    if not devices.revoke(store, did):
        raise HTTPException(404, "device not found")
    return {"ok": True}


# ---- 操作审计（任务87，仅管理员）----
@router.get("/audit", dependencies=[Depends(require_admin)])
def audit_query(limit: int = 50, offset: int = 0, action: str | None = None,
                actor: str | None = None, start_ts: str | None = None,
                end_ts: str | None = None):
    items, total = store.audit(limit=limit, offset=offset, action=action,
                               actor=actor, start_ts=start_ts, end_ts=end_ts)
    return {"items": items, "total": total}


# ---- 日志查询（任务93，读保护口径；只尾部/固定白名单/脱敏）----
_LOG_FILES = {"server.log", "server.log.1", "server.log.2", "server.log.3"}


@router.get("/logs", dependencies=[Depends(require_read)])
def query_logs(tail: int = 200, level: str | None = None, q: str | None = None,
               file: str = "server.log"):
    """只读日志尾部查询（最多 1000 行，脱敏 URL 凭证，固定白名单）。"""
    from .redact import redact_value
    if file not in _LOG_FILES:
        raise HTTPException(400, f"file 只允许: {'|'.join(sorted(_LOG_FILES))}")
    tail = max(1, min(tail, 1000))
    level_upper = level.upper() if level else None
    if level_upper and level_upper not in ("ERROR", "WARNING", "INFO"):
        raise HTTPException(400, "level 只允许 ERROR|WARNING|INFO")
    q_clean = (q or "").strip()[:100]
    log_path = settings.data_dir / "logs" / file
    if not log_path.exists():
        return {"file": file, "lines": [], "matched": 0, "scanned": 0,
                "note": "日志文件不存在（可能 LS_LOG_FILE=0）"}
    # 只从尾部读最多 2MB
    fsize = log_path.stat().st_size
    read_bytes = min(fsize, 2 * 1024 * 1024)
    with open(log_path, "rb") as f:
        if read_bytes < fsize:
            f.seek(fsize - read_bytes)
        raw = f.read()
    all_lines = raw.decode("utf-8", errors="replace").splitlines()
    scanned = len(all_lines)
    out, matched = [], 0
    for ln in reversed(all_lines):
        red = redact_value(ln)
        if level_upper and f" {level_upper} " not in f" {red} ":
            continue
        if q_clean and q_clean.lower() not in red.lower():
            continue
        matched += 1
        if matched <= tail:
            out.append(red)
    out.reverse()  # 正序返回最近 tail 条
    return {"file": file, "lines": out, "matched": matched, "scanned": scanned,
            "truncated": read_bytes < fsize}


# ---- Prometheus 指标（任务94，读保护口径；text/plain）----
@router.get("/metrics", dependencies=[Depends(require_read)])
def prometheus_metrics():
    """Prometheus 文本指标（无敏感标签：只 camera_id/level/status）。"""
    from . import logging_setup as ls
    lines = []
    try:
        lines += ["# HELP labsentinel_up 1=alive", "# TYPE labsentinel_up gauge",
                  "labsentinel_up 1"]
        lines.append(f"labsentinel_uptime_seconds {ls.uptime_s()}")
    except Exception:  # noqa: BLE001
        pass
    try:
        cams = store.cameras()
        lines.append(f"labsentinel_cameras_total {len(cams)}")
        running = 0
        st = manager.status() if manager else {"cameras": []}
        st_by_id = {c["id"]: c for c in st.get("cameras", [])}
        for c in cams:
            s = st_by_id.get(c["id"], {})
            if s.get("status") == "running":
                running += 1
            cid = c["id"]
            if "fps" in s:
                lines.append(f'labsentinel_camera_fps{{camera_id="{cid}"}} {s["fps"]}')
            if s.get("frame_age_s") is not None:
                lines.append(f'labsentinel_camera_frame_age_seconds{{camera_id="{cid}"}} {s["frame_age_s"]}')
        lines.append(f"labsentinel_cameras_running {running}")
        # T-336：迁移期观测——按分析器实现（python|rust）打标签，切换前后对比事件率/fps
        for impl in ("python", "rust"):
            n = sum(1 for c in cams if (c.get("analyzer") or settings.analyzer_default) == impl)
            lines.append(f'labsentinel_cameras{{analyzer="{impl}"}} {n}')
        eng_state = "not_loaded"
        try:
            from . import engine as _e
            eng_state = str(_e.PRELOAD.get("state") or "not_loaded")
        except Exception:  # noqa: BLE001
            pass
        lines.append(f'labsentinel_engine_state{{state="{eng_state}"}} 1')
        lines.append(f"labsentinel_events_total {store.count_events()}")
    except Exception:  # noqa: BLE001
        pass
    try:
        for lv, n in (store.today_event_counts() or {}).items():
            lines.append(f'labsentinel_events_today{{level="{lv}"}} {n}')
    except Exception:  # noqa: BLE001
        pass
    try:
        for status_key, n in (dispatcher.store.delivery_counts() if dispatcher.store else {}).items():
            lines.append(f'labsentinel_alert_deliveries{{status="{status_key}"}} {n}')
    except Exception:  # noqa: BLE001
        pass
    try:
        import shutil
        free = shutil.disk_usage(str(settings.data_dir)).free
        lines.append(f"labsentinel_disk_free_bytes {free}")
    except Exception:  # noqa: BLE001
        pass
    try:
        db = settings.db_path
        if Path(str(db)).exists():
            lines.append(f"labsentinel_db_size_bytes {Path(str(db)).stat().st_size}")
    except Exception:  # noqa: BLE001
        pass
    try:
        lines.append(f"labsentinel_schema_version {store.schema_version()}")
    except Exception:  # noqa: BLE001
        pass
    from fastapi.responses import Response
    return Response(content="\n".join(lines) + "\n", media_type="text/plain; version=0.0.4")


# ---- 未处置升级提醒（任务82）----
@router.get("/escalation/status", dependencies=[Depends(require_read)])
def escalation_status():
    from . import escalation
    return escalation.status(store)


# ---- 配置校验（任务95，仅管理员）----
@router.get("/config/validate", dependencies=[Depends(require_admin)])
def config_validate():
    from .validate import validate_settings
    return {"items": validate_settings(),
            "fail": sum(1 for r in validate_settings() if r["level"] == "FAIL"),
            "warn": sum(1 for r in validate_settings() if r["level"] == "WARN")}


# ---- 告警文案模板（任务85）：预览不发送不写库 ----
@router.get("/alert-template", dependencies=[Depends(require_read)])
def alert_template_get():
    return {
        "title": settings.alert_template_title,
        "text": settings.alert_template_text,
        "placeholders": {
            "icon": "级别图标（ℹ️/⚠️/🔴）", "level": "级别数字 1-3",
            "type": "事件类型标识", "type_text": "事件类型中文名",
            "ts": "事件时间", "camera_id": "摄像头 ID",
            "track_id": "跟踪 ID（occupancy 为 0）", "detail": "事件详情",
            "track_part": "「 · track N」或空串（occupancy 时为空）",
        },
    }


@router.post("/alert-template/preview", dependencies=[Depends(require_admin)])
def alert_template_preview(body: dict):
    from .alerts import alert_title, render_alert
    row = {"ts": body.get("ts") or datetime.now().isoformat(timespec="seconds"),
           "camera_id": int(body.get("camera_id", 1)),
           "type": body.get("type", "enter"),
           "level": int(body.get("level", 2)),
           "track_id": int(body.get("track_id", 1)),
           "detail": body.get("detail", "（示例事件详情）")}
    return {"title": alert_title(row["level"]), "text": render_alert(row)}


# ---- 告警静默（任务81）：检测/事件照常，只抑制外部渠道；到期自动恢复 ----
_ALLOWED_SILENCE_SCOPES = ("global", "camera", "level", "type")


@router.get("/alert-silences", dependencies=[Depends(require_read)])
def list_alert_silences(active: int = 1):
    out = []
    for s in store.silences(active_only=bool(active)):
        try:
            remain = int((datetime.fromisoformat(s["until_ts"])
                          - datetime.now()).total_seconds())
        except ValueError:
            remain = None
        out.append({**s, "remaining_s": max(0, remain) if remain is not None else None})
    return out


@router.post("/alert-silences", dependencies=[Depends(require_admin)])
def add_alert_silence(body: dict):
    scope = body.get("scope", "global")
    if scope not in _ALLOWED_SILENCE_SCOPES:
        raise HTTPException(400, f"scope 必须为 {'|'.join(_ALLOWED_SILENCE_SCOPES)}")
    target = body.get("target")
    if scope == "global":
        target = None
    else:
        if target is None or str(target) == "":
            raise HTTPException(400, f"scope={scope} 需要 target")
        if scope == "camera" and store.camera(int(target)) is None:
            raise HTTPException(404, "camera not found")
        if scope == "level" and int(target) not in (1, 2, 3):
            raise HTTPException(400, "level 必须为 1|2|3")
        if scope == "type" and target not in TYPE_TEXT:
            raise HTTPException(400, f"type 必须为 {'|'.join(TYPE_TEXT)}")
    duration = int(body.get("duration_s", 3600))
    if not (60 <= duration <= 7 * 86400):
        raise HTTPException(400, "duration_s 必须 60–604800（1 分钟 ~ 7 天）")
    rec = store.add_silence(scope, str(target) if target is not None else None,
                            body.get("reason"), created_by="admin", duration_s=duration)
    return rec


@router.delete("/alert-silences/{sid}", dependencies=[Depends(require_admin)])
def delete_alert_silence(sid: int):
    if not store.delete_silence(sid):
        raise HTTPException(404, "silence not found")
    return {"ok": True}


# ---- cameras ----
# 2026-10-30 审阅：读保护口径统一——/api/system 与 /api/health/* 例外永不拦截（启动器健康检查用），
# 其余 GET（cameras/zones/rules/snapshot/webrtc/stats/maintenance/export）一律 require_read。
# 此前 export/evidence.zip、实时快照等含敏感数据的端点漏挂，读保护形同虚设。
@router.get("/cameras", dependencies=[Depends(require_read)])
def cameras():
    return store.cameras()


# ---- 版本更新检查（任务117：只提示，不下载/安装/执行）----
@router.get("/update/check", dependencies=[Depends(require_read)])
def update_check_route(refresh: int = 0):
    return update_check.check_update(
        settings.update_manifest_url, _system_version(),
        allow_http=settings.update_allow_http,
        timeout=settings.update_timeout_s, refresh=bool(refresh))


# ---- 插件（外挂）一期（任务126；全部 require_admin：插件=代码执行）----
@router.get("/plugins", dependencies=[Depends(require_admin)])
def plugins_list():
    return {"plugins": plugins_mod.manager.scan()}


def _mgr():
    if plugins_mod.manager is None:
        raise HTTPException(503, "插件管理未初始化")
    return plugins_mod.manager


@router.post("/plugins/{pid}/enable", dependencies=[Depends(require_admin)])
def plugin_enable(pid: str):
    if not _mgr().enable(pid):
        raise HTTPException(404, "插件不存在或清单非法")
    return {"id": pid, "enabled": True}


@router.post("/plugins/{pid}/disable", dependencies=[Depends(require_admin)])
def plugin_disable(pid: str):
    if not _mgr().disable(pid):
        raise HTTPException(404, "插件不存在或清单非法")
    return {"id": pid, "enabled": False}


@router.post("/plugins/{pid}/start", dependencies=[Depends(require_admin)])
def plugin_start(pid: str):
    ok, msg = _mgr().start(pid)
    if not ok:
        raise HTTPException(400, msg)
    return {"id": pid, "status": "started", "detail": msg}


@router.post("/plugins/{pid}/stop", dependencies=[Depends(require_admin)])
def plugin_stop(pid: str):
    ok, msg = _mgr().stop(pid)
    if not ok:
        raise HTTPException(400, msg)
    return {"id": pid, "status": "stopped", "detail": msg}


@router.get("/plugins/{pid}/logs", dependencies=[Depends(require_admin)])
def plugin_logs(pid: str, tail: int = 200):
    return {"id": pid, "logs": _mgr().logs(pid, tail=tail)}


# ---- MQTT 事件总线状态（任务125；未配置也 200）----
@router.get("/mqtt/status", dependencies=[Depends(require_read)])
def mqtt_status():
    return mqtt_bridge.status()


# ---- 事件时间线（任务136：Frigate review 式热力条数据源；只读聚合）----
@router.get("/events/timeline", dependencies=[Depends(require_read)])
def events_timeline(days: int = 7, camera_id: int | None = None, level: int | None = None):
    if not 1 <= days <= 30:
        raise HTTPException(400, "days 必须在 1-30")
    return stats_timeline_query(store, days=days, camera_id=camera_id, level=level)


# ---- GenAI 复核状态（任务124；只读计数，不含任何截图/内容）----
@router.get("/genai/status", dependencies=[Depends(require_read)])
def genai_status():
    return genai.status()


# ---- 局域网发现助手（任务113；require_admin：发现=内网动作）----
@router.get("/discovery/cameras", dependencies=[Depends(require_admin)])
def discovery_cameras(timeout: float = 3.0, refresh: int = 0):
    """WS-Discovery 组播发现 ONVIF 设备（30s 缓存，refresh=1 强制重发）。

    无设备/组播不可用都返回 200 + 可读 error 文案（不 500）。
    """
    timeout = min(max(timeout, 1.0), 5.0)
    try:
        devices = discovery.get_devices(timeout=timeout, refresh=bool(refresh))
    except Exception as e:  # noqa: BLE001
        return {"devices": [], "lan": discovery.lan_addresses(),
                "error": f"发现失败：{e!r}"}
    hint = ("" if devices else
            "未发现设备：确认摄像头与主机同网段、摄像头已开启 ONVIF、UDP 3702 未被防火墙拦截；"
            "也可用品牌模板手输 RTSP 地址")
    return {"devices": devices, "lan": discovery.lan_addresses(), "error": hint}


@router.post("/discovery/probe", dependencies=[Depends(require_admin)])
def discovery_probe(body: dict):
    """TCP/ffprobe 只读探测（凭证仅用于本次探测，不写日志/不回传）。"""
    host = str(body.get("host") or "").strip()
    if not host:
        raise HTTPException(400, "host 必填")
    try:
        port = int(body.get("port") or 554)
    except (TypeError, ValueError):
        raise HTTPException(400, "port 必须是整数")
    if not 1 <= port <= 65535:
        raise HTTPException(400, "port 必须在 1-65535")
    return discovery.probe_host(host, port=port,
                                username=str(body.get("username") or "") or None,
                                password=str(body.get("password") or "") or None)


# ---- 分组与批量启停（任务111）----
@router.put("/cameras/{cid}/group", dependencies=[Depends(require_admin)])
def set_camera_group(cid: int, body: dict):
    name = str(body.get("group_name") or "")
    if len(name) > 32:
        raise HTTPException(400, "group_name 最长 32 字符")
    if store.camera(cid) is None:
        raise HTTPException(404, "camera not found")
    store.set_camera_group(cid, name)
    return {"id": cid, "group_name": name}


@router.get("/camera-groups", dependencies=[Depends(require_read)])
def camera_groups():
    return {"groups": store.camera_groups()}


@router.post("/cameras/bulk", dependencies=[Depends(require_admin)])
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
        cam = store.camera(cid)
        if cam is None:
            missing.append(cid)
            continue
        if manager is None:
            failed.append({"id": cid, "error": "manager not ready"})
            continue
        try:
            if action == "start":
                manager.start_camera(cam)
            else:
                manager.stop_camera(cid)
        except Exception as e:  # noqa: BLE001
            failed.append({"id": cid, "error": repr(e)})
    return {"ok": [i for i in ids if i not in missing and i not in [f["id"] for f in failed]],
            "missing": missing, "failed": failed}


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
    if body.type not in ("rtsp", "file", "usb", "folder"):
        raise HTTPException(400, "type must be rtsp|file|usb|folder")
    source = (body.source or "").strip()
    # T-103：source 白名单已在 CameraCreate 校验器执行（CameraCreate 返回规范化后的 source）
    if body.type == "folder":
        # T-111：允许根/系统目录黑名单（符号链接逃逸由 check_folder_root 的 resolve 处理）
        from .sources import SourceError, check_folder_root
        try:
            source = check_folder_root(source, settings.folder_allowed_roots)
            data_root = os.path.normcase(str(Path(settings.data_dir).resolve()))
            if os.path.normcase(source) == data_root or \
                    os.path.normcase(source).startswith(data_root + os.sep):
                raise HTTPException(400, "图片文件夹不能使用程序数据目录（会造成证据图自激识别）")
        except SourceError as e:
            raise HTTPException(400, str(e))
    cid = store.add_camera(body.name, source, body.type, group_name=body.group_name)
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
    # 图片文件夹：删记录 + 派生证据/缩略图（原始截图永不删除）
    removed = store.delete_folder_images_by_camera(cid)
    for r in removed:
        # 事件仍引用同一证据副本时保留文件（事件保留期 180 天，查看证据不能 404）
        if not (r.get("snapshot") and store.event_snapshot_referenced(r["snapshot"])):
            _unlink_data_file(r.get("snapshot"))
        _unlink_data_file(str(Path(settings.data_dir) / "thumbs" / f"img_{r['id']}.jpg"))
    store.delete_camera(cid)
    go2rtc.sync_streams(store)  # 实时画面：同步流媒体网关配置
    return {"ok": True}


def _unlink_data_file(path: str | None) -> None:
    """只删除 data_dir 内的派生文件（证据/缩略图），路径校验防误删。"""
    if not path:
        return
    try:
        p = Path(str(path))
        real = os.path.normcase(str(p.resolve()))
        base = os.path.normcase(str(Path(settings.data_dir).resolve()))
        if (real == base or real.startswith(base + os.sep)) and p.is_file():
            p.unlink()
    except (OSError, RuntimeError, ValueError):
        pass


@router.get("/cameras/{cid}/webrtc", dependencies=[Depends(require_read)])
def camera_webrtc(cid: int):
    """T-102：返回经 FastAPI 鉴权代理的相对 WS 路径（浏览器不再直连 go2rtc :1984）。

    前端按 location 组装 ws/wss（http→ws、https→wss），令牌走 Sec-WebSocket-Protocol
    子协议（T-106），媒体面仍在 go2rtc :8555（webrtc.candidates 可配跨网段地址）。
    """
    cam = store.camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    if cam["type"] == "folder":
        raise HTTPException(400, "图片文件夹源没有实时流；请在「图片识别」页查看关键图片")
    return {"camera_id": cid, "ws_url": f"/ws/webrtc/{cid}"}


@router.post("/cameras/{cid}/copy-config", dependencies=[Depends(require_admin)])
def copy_camera_config(cid: int, body: dict):
    """把源摄像头的区域/规则批量复制到多路目标（任务80）。

    先全量校验（源/目标存在、排除自身、目标去重）再写库；规则 zone_id 按**区域名**映射到
    目标（name_suffix 参与命名）；copy_zones=False 时规则若无同名目标区域则跳过并 warning
    （绝不指向源摄像头的区域）。不启动摄像头、不复制事件。
    """
    src = store.camera(cid)
    if src is None:
        raise HTTPException(404, "camera not found")
    targets = body.get("target_ids") or []
    if not targets:
        raise HTTPException(400, "target_ids 不能为空")
    targets = sorted(set(int(t) for t in targets))
    if cid in targets:
        raise HTTPException(400, "target_ids 不能包含源摄像头自身")
    missing = [t for t in targets if store.camera(t) is None]
    if missing:
        raise HTTPException(404, f"目标摄像头不存在: {missing}")
    copy_zones = bool(body.get("copy_zones", True))
    copy_rules = bool(body.get("copy_rules", True))
    suffix = str(body.get("name_suffix", "") or "")
    src_zones = store.zones(cid)
    src_rules = store.rules(cid)
    warnings: list[str] = []
    zones_created = zones_skipped = rules_created = rules_skipped = 0
    targets_out = []
    for tid in targets:
        zone_map: dict[str, int] = {}
        z0, r0 = zones_skipped, rules_skipped  # 每目标跳过数 = 前后差值
        tgt_zone_names = {z["name"]: z["id"] for z in store.zones(tid)}
        if copy_zones:
            for z in src_zones:
                new_name = z["name"] + suffix
                if new_name in tgt_zone_names:
                    zone_map[z["name"]] = tgt_zone_names[new_name]  # 已有同名：映射给规则用
                    zones_skipped += 1
                    continue
                zid = store.add_zone(tid, new_name, z["polygon"])
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
                          and x["time_end"] == r["time_end"] for x in store.rules(tid))
                if dup:
                    rules_skipped += 1
                    continue
                store.add_rule({"camera_id": tid, "zone_id": zid, "behavior": r["behavior"],
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


@router.get("/cameras/{cid}/status", dependencies=[Depends(require_read)])
def camera_status(cid: int):
    """单路运行指标（任务65）：帧龄/最近错误（已脱敏）/重连次数/累计帧数。"""
    cam = store.camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    w = manager.workers.get(cid) if manager else None
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
    if manager is not None:
        out.update(manager.supervisor_info(cid))  # 任务114：restart_count/守护状态
    return out


# ---- 配置导入导出（任务79）：名称关联、默认脱敏、仅管理员 ----
_CONFIG_FORMAT = "labsentinel-config"
_RTSP_CRED = re.compile(r"(//)([^/@\s:]+):([^@\s]+)@")


@router.get("/config/export", dependencies=[Depends(require_admin)])
def config_export(with_secrets: int = 0):
    """配置级导出（不含事件/截图/人脸）。默认把 RTSP 凭证脱敏为 ***:***@；
    with_secrets=1 返回真实 source（require_admin 已要求令牌；响应 no-store）。"""
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
        from .sources import SourceError, validate_source
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


@router.post("/config/import", dependencies=[Depends(require_admin)])
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
        if manager is not None:
            manager.stop_all()  # 引用现有摄像头的 worker 先停
        store.clear_config()
    created = {"cameras": 0, "zones": 0, "rules": 0}
    skipped = {"cameras": [], "zones": [], "rules": []}
    warns: list[str] = []
    cam_id_by_name = {c["name"]: c["id"] for c in store.cameras()}
    existing_zones = {(z["camera_id"], z["name"]) for z in store.zones()}
    for c in cams:
        if c["name"] in cam_id_by_name:
            skipped["cameras"].append(c["name"])
            continue
        cam_id_by_name[c["name"]] = store.add_camera(c["name"], c["source"], c["type"])
        created["cameras"] += 1
        if c["type"] == "folder" and not Path(str(c["source"])).is_dir():
            warns.append(f"图片文件夹不存在：{c['name']} → {c['source']}（启动后为 error，"
                         "路径恢复后会自动重试）")
    for z in zones:
        cid = cam_id_by_name[z["camera_name"]]
        if (cid, z["name"]) in existing_zones:
            skipped["zones"].append(z["name"])
            continue
        zid = store.add_zone(cid, z["name"], z["polygon"])
        existing_zones.add((cid, z["name"]))
        created["zones"] += 1
    # 重建 zone_name → (camera_name, zid) 映射（含 merge 已有区域）
    zkey_to_zid = {}
    zname_by_id = {z["id"]: z["name"] for z in store.zones()}
    cam_name_by_id = {c["id"]: c["name"] for c in store.cameras()}
    for z in store.zones():
        zkey_to_zid[(cam_name_by_id.get(z["camera_id"]), z["name"])] = z["id"]
    existing_rules = {(cam_name_by_id.get(r["camera_id"]), r["behavior"],
                       zname_by_id.get(r["zone_id"]), r["time_start"], r["time_end"])
                      for r in store.rules()}
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
        store.add_rule({"camera_id": cid, "zone_id": zid, "behavior": r["behavior"],
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


# ---- 图片文件夹识别（任务121）----
def _folder_media_path(row: dict) -> Path:
    """把 folder_images 行还原为可读取的源文件路径，并做目录/扩展名安全校验。

    只允许读取该摄像头接入目录内的图片：防 DB 被篡改/软链接指向系统敏感文件。
    """
    cam = store.camera(row["camera_id"])
    if cam is None:
        raise HTTPException(404, "camera not found")
    p = Path(str(row["path"]))
    if p.suffix.lower() not in image_exts():
        raise HTTPException(415, "不支持的图片类型")
    if not p.is_file():
        raise HTTPException(404, "图片文件不存在（可能已被移动或删除）")
    try:
        real = os.path.normcase(str(p.resolve()))
        root = os.path.normcase(str(Path(str(cam["source"])).resolve()))
        if os.path.commonpath([real, root]) != root:
            raise HTTPException(403, "图片路径不在接入文件夹内")
    except ValueError:
        raise HTTPException(403, "图片路径不在接入文件夹内")
    except (OSError, RuntimeError):
        raise HTTPException(404, "图片文件不可访问")
    return p


def _folder_readable_path(row: dict) -> Path:
    """优先原始截图；源文件已被外部软件轮转删除时，回退到违规证据副本。"""
    try:
        return _folder_media_path(row)
    except HTTPException as e:
        snap = row.get("snapshot")
        if e.status_code != 404 or not snap:
            raise
        try:
            sp = Path(str(snap))
            real = os.path.normcase(str(sp.resolve()))
            base = os.path.normcase(str(Path(settings.data_dir).resolve()))
            if (real == base or real.startswith(base + os.sep)) and sp.is_file():
                return sp
        except (OSError, RuntimeError, ValueError):
            pass
        raise


def _folder_row_out(row: dict) -> dict:
    out = dict(row)
    out["thumb_url"] = f"/api/folder-images/{row['id']}/thumb"
    out["file_url"] = f"/api/folder-images/{row['id']}/file"
    out["has_snapshot"] = bool(row.get("snapshot"))
    return out


@router.get("/folder-images/summary", dependencies=[Depends(require_read)])
def folder_images_summary(days: int = 7, camera_id: int | None = None):
    """图片识别汇总（近 N 天，按摄像头）：汇报卡片与"关键/违规"趋势。"""
    days = max(1, min(int(days), 3650))
    cams = {c["id"]: c for c in store.cameras()}
    rows = store.folder_image_summary(days=days, camera_id=camera_id)
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


@router.get("/folder-images", dependencies=[Depends(require_read)])
def folder_images(camera_id: int | None = None, key: int | None = None,
                  violation: int | None = None, start_ts: str | None = None,
                  end_ts: str | None = None, error_only: int = 0,
                  limit: int = 60, offset: int = 0):
    """关键图片列表（默认全部，key=1 只看有人的图，violation=1 只看违规图）。"""
    limit = max(1, min(int(limit), 500))
    offset = max(0, int(offset))
    rows = store.folder_images(camera_id=camera_id, key=key, violation=violation,
                               start_ts=start_ts, end_ts=end_ts, error_only=bool(error_only),
                               limit=limit, offset=offset)
    total = store.count_folder_images(camera_id=camera_id, key=key, violation=violation,
                                      start_ts=start_ts, end_ts=end_ts,
                                      error_only=bool(error_only))
    names = {c["id"]: c["name"] for c in store.cameras()}
    return {"items": [_folder_row_out(r) for r in rows], "total": total,
            "limit": limit, "offset": offset,
            "camera_names": names}


def _csv_cell(value) -> str:
    """CSV 单元格防公式注入（T-502：与 export 共用 csvsafe，防两套规则漂移）。"""
    from .csvsafe import safe_cell
    return safe_cell(value)


@router.get("/folder-images/export.csv", dependencies=[Depends(require_read)])
def export_folder_images_csv(camera_id: int | None = None, key: int | None = None,
                             violation: int | None = None, start_ts: str | None = None,
                             end_ts: str | None = None, error_only: int = 0):
    """按当前筛选导出图片识别记录 CSV（UTF-8 BOM，Excel 直接打开）。最多 5 万行。"""
    rows: list[dict] = []
    offset = 0
    while len(rows) < 50000:
        batch = store.folder_images(camera_id=camera_id, key=key, violation=violation,
                                    start_ts=start_ts, end_ts=end_ts,
                                    error_only=bool(error_only), limit=500, offset=offset)
        rows.extend(batch)
        if len(batch) < 500:
            break
        offset += 500
    names = {c["id"]: c["name"] for c in store.cameras()}
    buf = io.StringIO()
    w = csv.writer(buf)
    w.writerow(["摄像头", "图片时间", "识别时间", "文件名", "源路径", "人数", "在区",
                "未穿", "已穿", "命中区域", "关键", "违规", "违规明细", "关联事件", "读取错误"])
    for r in rows:
        w.writerow([_csv_cell(v) for v in [
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


@router.get("/folder-images/{iid}", dependencies=[Depends(require_read)])
def folder_image_detail(iid: int):
    row = store.folder_image(iid)
    if row is None:
        raise HTTPException(404, "image record not found")
    return _folder_row_out(row)


@router.get("/folder-images/{iid}/file", dependencies=[Depends(require_read)])
def folder_image_file(iid: int):
    row = store.folder_image(iid)
    if row is None:
        raise HTTPException(404, "image record not found")
    p = _folder_readable_path(row)
    mt = mimetypes.guess_type(str(p))[0] or "application/octet-stream"
    return FileResponse(p, media_type=mt, headers={"Cache-Control": "private, max-age=3600"})


@router.get("/folder-images/{iid}/thumb", dependencies=[Depends(require_read)])
def folder_image_thumb(iid: int):
    """列表缩略图（480px，磁盘缓存）；原图走 /file，避免首屏拉一堆 4K 截图。"""
    row = store.folder_image(iid)
    if row is None:
        raise HTTPException(404, "image record not found")
    p = _folder_readable_path(row)
    data = thumbnail_bytes(str(p), iid)
    if data is None:
        raise HTTPException(415, "缩略图生成失败（图片损坏或格式不支持）")
    from fastapi.responses import Response
    return Response(content=data, media_type="image/jpeg",
                    headers={"Cache-Control": "private, max-age=3600"})


@router.post("/folders/{cid}/scan", dependencies=[Depends(require_admin)])
def folder_scan(cid: int):
    """立即扫描：运行中的文件夹源唤醒一轮；未运行则启动该路接入。"""
    cam = store.camera(cid)
    if cam is None:
        raise HTTPException(404, "camera not found")
    if cam["type"] != "folder":
        raise HTTPException(400, "该摄像头不是图片文件夹源")
    if manager.wake_camera(cid):
        return {"ok": True, "started": False, "msg": "已触发立即扫描"}
    manager.start_camera(cam)
    return {"ok": True, "started": True, "msg": "图片文件夹接入已启动"}


# ---- T-505 第三步（纯搬移）：zones/rules/templates/calendar 移入 routers/config.py ----
from .routers import config as _config_router  # noqa: E402
router.include_router(_config_router.router)


# ---- events ----
@router.get("/events", dependencies=[Depends(require_read)])
def events(limit: int = 50, camera_id: int | None = None, level: int | None = None,
           status: str | None = None, offset: int = 0,
           start_ts: str | None = None, end_ts: str | None = None,
           q: str | None = None, ai_verdict: str | None = None):
    if q is not None and len(q.strip()) > 100:  # 任务84：>100 字符拒绝；空白=不过滤
        raise HTTPException(400, "q 长度须 ≤ 100 字符")
    _check_ai_verdict(ai_verdict)  # 任务132
    limit = max(1, min(limit, 200))  # 任务19：上限钳制
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    return [_event_row(r) for r in
            store.events(limit=limit, camera_id=camera_id, level=level, status=status,
                         offset=offset, start_ts=start_ts, end_ts=end_ts, q=q,
                         ai_verdict=ai_verdict)]


@router.get("/events/count", dependencies=[Depends(require_read)])
def events_count(camera_id: int | None = None, level: int | None = None,
                 status: str | None = None,
                 start_ts: str | None = None, end_ts: str | None = None,
                 q: str | None = None, ai_verdict: str | None = None):
    """与 events 相同过滤条件的总数（任务19；分页用；任务84 q 关键词）。"""
    if q is not None and len(q.strip()) > 100:
        raise HTTPException(400, "q 长度须 ≤ 100 字符")
    _check_ai_verdict(ai_verdict)  # 任务132
    start_ts, end_ts = _normalize_ts_range(start_ts, end_ts)
    return {"count": store.count_events(camera_id=camera_id, level=level, status=status,
                                        start_ts=start_ts, end_ts=end_ts, q=q,
                                        ai_verdict=ai_verdict)}


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


@router.post("/events/{eid}/status", dependencies=[Depends(require_triage)])  # T-620
def set_event_status(eid: int, body: dict):
    """事件处置（任务12）：status ∈ acked|false_positive|ignored。

    标记误报时把截图**复制**到 annotation_pool/（不删原事件）——数据飞轮误报样本池。
    """
    status = (body or {}).get("status", "")
    if status not in EVENT_STATUSES or status == "new":
        raise HTTPException(400, f"status 必须是 {'|'.join(EVENT_STATUSES[1:])}")
    r = _triage_one(eid, status, (body or {}).get("handled_by", "admin"))
    if r == "missing":
        raise HTTPException(404, "event not found")
    return {"ok": True, "status": status}


@router.post("/events/bulk-status", dependencies=[Depends(require_admin)])
def bulk_event_status(body: dict):
    """批量处置（任务83）：逐条短事务复用单条逻辑；missing 容错；单条失败不中断整批。"""
    status = (body or {}).get("status", "")
    if status not in EVENT_STATUSES or status == "new":
        raise HTTPException(400, f"status 必须是 {'|'.join(EVENT_STATUSES[1:])}")
    ids = body.get("ids")
    if not isinstance(ids, list) or not ids:
        raise HTTPException(400, "ids 不能为空")
    ids = sorted({int(x) for x in ids})
    if len(ids) > 200:
        raise HTTPException(400, "ids 一次最多 200 条")
    handled_by = (body or {}).get("handled_by", "admin")
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


# ---- T-505 第二步（纯搬移）：face / alerts / maintenance 路由移入 routers/ops.py ----
# 挂在被搬移段的原位置 → 注册顺序与路径集合完全不变（OpenAPI 快照无漂移）。
from .routers import ops as _ops_router  # noqa: E402
router.include_router(_ops_router.router)
# 兼容别名：历史测试/调用方直接引用 api.<handler> 的入口（不重复注册路由）
alerts_config_get = _ops_router.alerts_config_get
alerts_config_put = _ops_router.alerts_config_put
alerts_config_reset = _ops_router.alerts_config_reset



# ---- T-505 第一步（纯搬移）：stats / export 路由移入 routers/stats.py ----
# 挂在被搬移段的原位置 → 注册顺序与路径集合完全不变（OpenAPI 快照无漂移）。
from .routers import stats as _stats_router  # noqa: E402
router.include_router(_stats_router.router)

# ---- websocket ----（T-106 / Q-03：口径统一 + 强制同源 + 子协议传令牌）
WS_ORIGIN_CODE = 4403


def _ws_origin_ok(ws: WebSocket) -> bool:
    """存在 Origin 时必须与 Host 同源或在 LS_ALLOWED_ORIGINS 白名单（防 CSWSH）。"""
    origin = ws.headers.get("origin")
    if not origin:
        return True  # 非浏览器客户端（curl/插件/移动端）
    host = ws.headers.get("host", "")
    if origin.split("://", 1)[-1] == host:
        return True
    allowed = [o.strip() for o in settings.allowed_origins.split(",") if o.strip()]
    return origin in allowed


def _ws_token(ws) -> str | None:
    """令牌来源优先级：Sec-WebSocket-Protocol: bearer.<token> → ?token=（弃用，日志告警）。"""
    protos = [p.strip() for p in ws.headers.get("sec-websocket-protocol", "").split(",")]
    for p in protos:
        if p.startswith("bearer."):
            return p[len("bearer."):]
    t = ws.query_params.get("token")
    if t:
        print("[auth] WS 仍使用 ?token= 查询参数（弃用，改用子协议 bearer.<token>）", flush=True)
    return t


def _ws_auth_check(ws) -> bool:
    """WS 握手鉴权：Origin 校验 + 与 HTTP 读口径一致（auth_read_protected=false 不要求令牌）。"""
    if not _ws_origin_ok(ws):
        return False
    if settings.auth_read_protected:
        return _read_token_ok(_ws_token(ws))
    return True


async def ws_events(ws: WebSocket):
    if not _ws_origin_ok(ws):
        await ws.close(code=WS_ORIGIN_CODE)
        return
    if settings.auth_read_protected and not _read_token_ok(_ws_token(ws)):
        await ws.close(code=WS_AUTH_CODE)
        return
    # T-201/B-19：try/finally 保证任何异常都清理 hub；子协议回显（客户端发了 labsentinel 时）
    protos = [p.strip() for p in ws.headers.get("sec-websocket-protocol", "").split(",")]
    subprotocol = "labsentinel" if "labsentinel" in protos else None
    await dispatcher.hub.connect(ws, subprotocol=subprotocol)
    try:
        while True:
            await ws.receive_text()  # 忽略客户端心跳
    except WebSocketDisconnect:
        pass
    finally:
        dispatcher.hub.disconnect(ws)

