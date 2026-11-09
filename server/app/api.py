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
from .models import (CameraCreate, ClaimPairingBody, DiscoveryProbeBody, RuleCreate,
                     SilenceCreate, TemplatePreviewBody, ZoneCreate)
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


def _auth_off() -> bool:
    """Q-02/R2-102 逃生口：LS_AUTH_DISABLED=1 且绑定回环时关闭鉴权。

    绑定地址一律读 settings.host（pydantic-settings 同时读环境变量与 .env），
    不再读 os.environ["LS_HOST"]——那样会漏掉 .env 里的值，且与实际监听脱钩。
    validate.py 的 FATAL + main._enforce_bind 保证非回环不会走到这里。
    """
    if not settings.auth_disabled:
        return False
    from .validate import is_loopback_host
    return is_loopback_host(settings.host)


def _token_ok(provided: str | None) -> bool:
    """管理令牌校验（写操作专用，任务88 语义收紧）。"""
    if not settings.admin_token or _auth_off():
        return True  # 未启用鉴权 / Q-02 逃生口（仅回环）
    if not provided:
        return False
    import hmac
    return hmac.compare_digest(provided, settings.admin_token)


def _read_token_ok(provided: str | None) -> bool:
    """R2-103/N-02 读令牌校验：admin/read/已配对设备令牌均可。

    77 号 Q-11 口径「设备令牌 = 查看 + 处置」：局域网绑定会强制开启读保护，
    若此处不接受设备令牌，配对后的手机在唯一需要它的场景里拿不到任何数据。
    """
    if not settings.admin_token or _auth_off():
        return True  # 未启用鉴权 / Q-02 逃生口（仅回环）
    if not provided:
        return False
    import hmac
    if hmac.compare_digest(provided, settings.admin_token):
        return True
    if settings.read_token and hmac.compare_digest(provided, settings.read_token):
        return True
    from . import devices
    return store is not None and devices.verify(store, provided) is not None


def _warn_same_token():
    """任务88：read_token 与 admin_token 相同 → 弱隔离 WARNING（允许运行）。"""
    if settings.admin_token and settings.read_token and \
            settings.admin_token == settings.read_token:
        print("[auth] ⚠ LS_READ_TOKEN 与 LS_ADMIN_TOKEN 相同——只读令牌形同虚设（弱隔离）")


_warn_same_token()


def _bearer_token(headers) -> str | None:
    """R2-103: mobile clients use `Authorization: Bearer <token>`; treat it as
    equivalent to X-Admin-Token when the latter is absent.
    """
    auth = headers.get("authorization", "")
    if auth.lower().startswith("bearer "):
        return auth[7:].strip()
    return None


def require_admin(request: Request):
    """写操作守卫：只接受管理令牌（任务88：只读令牌写永拒）。T-113：结果写入 state.actor。"""
    provided = request.headers.get("X-Admin-Token") or _bearer_token(request.headers)
    if not _token_ok(provided):
        request.state.actor = "rejected"
        if settings.admin_token and settings.read_token and provided == settings.read_token:
            raise HTTPException(401, "当前令牌无写权限（只读令牌）——请使用管理令牌")
        raise HTTPException(401, "需要管理令牌（X-Admin-Token 或 Authorization: Bearer）")
    request.state.actor = "admin" if settings.admin_token else "anonymous(auth-off)"


def require_triage(request: Request):
    """T-620/Q-11：事件处置守卫——管理令牌或设备令牌（可处置不可改配置）。"""
    if not settings.admin_token:
        request.state.actor = "anonymous(auth-off)"
        return
    provided = request.headers.get("X-Admin-Token") or _bearer_token(request.headers)
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
    provided = (request.headers.get("X-Admin-Token")
                or _bearer_token(request.headers))
    if settings.allow_url_token:
        provided = provided or request.query_params.get("token")
    if not _read_token_ok(provided):
        request.state.actor = "rejected"
        raise HTTPException(401, "需要管理令牌、设备令牌或签名媒体 URL")
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
def claim_pairing(body: ClaimPairingBody):
    """客户端用配对码换设备令牌（只返回一次；服务端只存 hash）。"""
    from . import devices
    code = body.code.strip()
    name = body.device_name[:64]
    platform = body.platform[:32]
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
def alert_template_preview(body: TemplatePreviewBody):
    from .alerts import alert_title, render_alert
    row = {"ts": body.ts or datetime.now().isoformat(timespec="seconds"),
           "camera_id": body.camera_id,
           "type": body.type,
           "level": body.level,
           "track_id": body.track_id,
           "detail": body.detail}
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
def add_alert_silence(body: SilenceCreate):
    scope = body.scope
    target = body.target
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
    duration = body.duration_s
    rec = store.add_silence(scope, str(target) if target is not None else None,
                            body.reason, created_by="admin", duration_s=duration)
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
def discovery_probe(body: DiscoveryProbeBody):
    """TCP/ffprobe 只读探测（凭证仅用于本次探测，不写日志/不回传）。"""
    host = body.host.strip()
    if not host:
        raise HTTPException(400, "host 必填")
    return discovery.probe_host(host, port=body.port,
                                username=body.username or None,
                                password=body.password or None)


# ---- T-505 第四步（纯搬移）：cameras 分组/批量/CRUD 与 config-io 移入子路由 ----
from .routers import cameras as _cameras_router  # noqa: E402
router.include_router(_cameras_router.router)
from .routers import configio as _configio_router  # noqa: E402
router.include_router(_configio_router.router)


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


def _csv_cell(value) -> str:
    """CSV 单元格防公式注入（T-502：与 export 共用 csvsafe，防两套规则漂移）。"""
    from .csvsafe import safe_cell
    return safe_cell(value)


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


# ---- T-505 第五步（纯搬移）：图片文件夹识别路由移入 routers/folders.py ----
from .routers import folders as _folders_router  # noqa: E402
router.include_router(_folders_router.router)


# ---- T-505 第三步（纯搬移）：zones/rules/templates/calendar 移入 routers/config.py ----
from .routers import config as _config_router  # noqa: E402
router.include_router(_config_router.router)


# ---- T-505 第六步（纯搬移）：events 路由移入 routers/events.py ----
from .routers import events as _events_router  # noqa: E402
router.include_router(_events_router.router)
# 兼容别名（历史测试直接调用）
events = _events_router.events
events_count = _events_router.events_count
set_event_status = _events_router.set_event_status
bulk_event_status = _events_router.bulk_event_status
_normalize_ts_range = _events_router._normalize_ts_range
_triage_one = _events_router._triage_one


# ---- T-505 第二步（纯搬移）：face / alerts / maintenance 路由移入 routers/ops.py ----
# 挂在被搬移段的原位置 → 注册顺序与路径集合完全不变（OpenAPI 快照无漂移）。
from .routers import ops as _ops_router  # noqa: E402
router.include_router(_ops_router.router)
# 兼容别名：历史测试/调用方直接引用 api.<handler> 的入口（不重复注册路由）
alerts_config_get = _ops_router.alerts_config_get
alerts_config_put = _ops_router.alerts_config_put
alerts_config_reset = _ops_router.alerts_config_reset
# 兼容别名（历史测试直接调用这些已移入子路由的 handler）
from .routers import cameras as _cameras_router  # noqa: E402
from .routers import configio as _configio_router  # noqa: E402
from .routers import folders as _folders_router  # noqa: E402
add_camera = _cameras_router.add_camera
delete_camera = _cameras_router.delete_camera
config_export = _configio_router.config_export
config_import = _configio_router.config_import
_validate_config_import = _configio_router._validate_config_import
folder_images_summary = _folders_router.folder_images_summary
folder_images = _folders_router.folder_images
export_folder_images_csv = _folders_router.export_folder_images_csv
folder_image_detail = _folders_router.folder_image_detail



# ---- T-505 第一步（纯搬移）：stats / export 路由移入 routers/stats.py ----
# 挂在被搬移段的原位置 → 注册顺序与路径集合完全不变（OpenAPI 快照无漂移）。
from .routers import stats as _stats_router  # noqa: E402
router.include_router(_stats_router.router)

# ---- Q-08 / T-406：实验服模型按需下载（许可确认 + SHA256 校验）----
from .routers import models as _models_router  # noqa: E402
router.include_router(_models_router.router)

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
    """R2-103 令牌来源优先级：子协议 bearer.<token> → Authorization: Bearer →
    ?token=（弃用，日志告警）。设备令牌走 _read_token_ok，与 HTTP 读口径一致。"""
    protos = [p.strip() for p in ws.headers.get("sec-websocket-protocol", "").split(",")]
    for p in protos:
        if p.startswith("bearer."):
            return p[len("bearer."):]
    bearer = _bearer_token(ws.headers)
    if bearer:
        return bearer
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


def accept_with_subprotocol(ws: WebSocket):
    """R2-104/N-05: echo the `labsentinel` subprotocol when the client offers it.

    RFC 6455 §4.1: a client that sends Sec-WebSocket-Protocol MUST close the
    connection if the server response omits it. /ws/events already did this;
    /ws/webrtc/{cid} did not -> with the first-run token enabled (default now)
    the browser dropped the signaling handshake and live video was dead.
    Shared here so both endpoints stay in sync.
    """
    protos = [p.strip() for p in ws.headers.get("sec-websocket-protocol", "").split(",")]
    subprotocol = "labsentinel" if "labsentinel" in protos else None
    return ws.accept(subprotocol=subprotocol)


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
    await dispatcher.hub.connect(ws, subprotocol=subprotocol)  # hub.connect 内部 accept
    try:
        while True:
            await ws.receive_text()  # 忽略客户端心跳
    except WebSocketDisconnect:
        pass
    finally:
        dispatcher.hub.disconnect(ws)

