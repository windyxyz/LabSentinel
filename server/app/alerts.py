"""告警通道：应用内 WebSocket + 企业微信 + 钉钉 + 飞书 + 微信(Server酱/PushPlus) + QQ(Qmsg酱) + SMTP 邮件。

所有 HTTP 推送渠道共用 _HttpPushChannel 基类：域名白名单 + 强制 https +
不跟随重定向（防 SSRF），文案统一走 render_alert。
"""
import asyncio
import concurrent.futures
import json
import smtplib
import socket
import ssl
import threading
import time
from collections import deque
from datetime import datetime
from email.mime.text import MIMEText
from urllib.parse import quote, urlparse

import httpx
from fastapi import WebSocket

from .config import settings
from .constants import LEVEL_ICON, TYPE_TEXT

# webhook 目标域名白名单（各渠道官方/常用服务 API），防误配/SSRF
ALLOWED_WEBHOOK_HOSTS = {"qyapi.weixin.qq.com"}
DINGTALK_ALLOWED_HOSTS = {"oapi.dingtalk.com"}
FEISHU_ALLOWED_HOSTS = {"open.feishu.cn"}
SERVERCHAN_ALLOWED_HOSTS = {"sctapi.ftqq.com"}       # Server酱 → 微信
PUSHPLUS_ALLOWED_HOSTS = {"www.pushplus.plus"}       # PushPlus → 微信
QMSG_ALLOWED_HOSTS = {"qmsg.zendee.cn"}              # Qmsg酱 → QQ


def render_alert(row: dict) -> str:
    """各推送渠道共用的告警文案（任务85：可经 LS_ALERT_TEMPLATE_TEXT 自定义模板）。

    占位符：icon/level/type/type_text/ts/camera_id/track_id/detail/track_part（track_part 为
    " · track N" 或空串，用于默认模板下 occupancy_over 不含 track 段——超出规格白名单的
    补充占位符，保证默认输出与历史版本逐字一致）。未知占位符原样保留并告警一次；
    detail 中的花括号不会被二次格式化（format_map 单次替换）。
    """
    level = row["level"]
    icon = LEVEL_ICON.get(level, "ℹ️")
    type_ = row["type"]
    type_text = TYPE_TEXT.get(type_, type_)
    track_id = row.get("track_id", 0)
    track_part = "" if type_ == "occupancy_over" else f" · track {track_id}"
    vals = {"icon": icon, "level": level, "type": type_, "type_text": type_text,
            "ts": row["ts"], "camera_id": row["camera_id"], "track_id": track_id,
            "track_part": track_part, "detail": row["detail"]}
    return settings.alert_template_text.format_map(_SafeDict(vals))


def alert_title(level: int) -> str:
    return settings.alert_template_title.format_map(_SafeDict({"level": level}))


class _SafeDict(dict):
    """format_map 用：未知占位符原样保留并告警一次（不抛异常）。"""

    def __missing__(self, key):
        if key not in _warned_placeholders:
            _warned_placeholders.add(key)
            print(f"[alert-template] 模板含未知占位符 {{{key}}}（原样保留；"
                  f"可用：icon/level/type/type_text/ts/camera_id/track_id/detail/track_part）")
        return "{" + key + "}"


_warned_placeholders: set = set()


class WebSocketHub:
    """事件实时推送。worker 线程通过 broadcast_sync 投递。"""

    def __init__(self):
        self._clients: list[WebSocket] = []
        self._loop: asyncio.AbstractEventLoop | None = None

    def bind_loop(self, loop):
        self._loop = loop

    async def connect(self, ws: WebSocket, subprotocol: str | None = None):
        # T-106：客户端通过子协议传令牌时回显 labsentinel（浏览器 WebSocket 要求）
        await ws.accept(subprotocol=subprotocol)
        self._clients.append(ws)

    def disconnect(self, ws: WebSocket):
        if ws in self._clients:
            self._clients.remove(ws)

    def broadcast_sync(self, payload: dict):
        """从非 async 上下文线程安全地广播。"""
        if self._loop is None or not self._clients:
            return
        try:
            asyncio.run_coroutine_threadsafe(self._broadcast(payload), self._loop)
        except RuntimeError:
            pass

    async def _broadcast(self, payload: dict):
        # T-201/B-19：并发发送 + 单客户端 2s 超时 + 异常统一清理（无残留连接）
        text = json.dumps(payload, ensure_ascii=False)
        clients = list(self._clients)

        async def one(ws):
            try:
                await asyncio.wait_for(ws.send_text(text), timeout=2.0)
            except Exception:  # noqa: BLE001
                self.disconnect(ws)

        await asyncio.gather(*(one(w) for w in clients), return_exceptions=True)


class _HttpPushChannel:
    """HTTP 推送渠道基类。transport 参数仅供单测注入 httpx.MockTransport。"""
    name = "push"
    allowed_hosts: frozenset = frozenset()

    def __init__(self, transport: httpx.BaseTransport | None = None):
        self._transport = transport

    def _check_url(self, url: str | None) -> str | None:
        if not url:
            return None
        p = urlparse(url)
        if p.scheme != "https" or p.hostname not in self.allowed_hosts:
            print(f"[{self.name}] webhook 被拒绝（仅允许 {sorted(self.allowed_hosts)} 的 https 地址）")
            return None
        return url

    def configured(self) -> bool:
        """是否完成配置。未配置的渠道在派发时整条跳过——不建投递记录、不重试、不产死信。"""
        return False

    def _post(self, url: str, **kwargs):
        try:
            if self._transport is not None:  # 测试路径
                with httpx.Client(transport=self._transport, follow_redirects=False) as client:
                    return client.post(url, timeout=5, **kwargs)
            return httpx.post(url, timeout=5, follow_redirects=False, **kwargs)
        except Exception as e:  # noqa: BLE001
            print(f"[{self.name}] send failed: {e}")
            return None

    @staticmethod
    def _is_ok(resp) -> bool:
        """投递成功判定：HTTP 2xx 且业务码正确（钉钉/企微 errcode、飞书 code、PushPlus code=200）。"""
        if resp is None or resp.status_code // 100 != 2:
            return False
        try:
            body = resp.json()
        except Exception:  # noqa: BLE001
            return True  # 非 JSON 的 2xx 视为成功（如文本网关）
        errcode = body.get("errcode", body.get("code", 0))
        return errcode in (0, 200)

    def send(self, level: int, text: str):
        raise NotImplementedError


class WeComChannel(_HttpPushChannel):
    name = "wecom"
    allowed_hosts = ALLOWED_WEBHOOK_HOSTS

    def configured(self) -> bool:
        return bool((settings.wecom_webhook or "").strip())

    def send(self, level: int, text: str) -> bool:
        url = self._check_url(settings.wecom_webhook)
        if not url:
            return False
        color = {1: "info", 2: "warning", 3: "error"}.get(level, "info")
        return self._is_ok(self._post(url, json={
            "msgtype": "markdown",
            "markdown": {"content": f'**LabSentinel 告警**\n> 级别: <font color="{color}">{level}</font>\n> {text}'},
        }))


class DingTalkChannel(_HttpPushChannel):
    """钉钉机器人：POST oapi.dingtalk.com/robot/send?access_token=...（markdown 消息）。"""
    name = "dingtalk"
    allowed_hosts = DINGTALK_ALLOWED_HOSTS

    def configured(self) -> bool:
        return bool((settings.dingtalk_webhook or "").strip())

    def send(self, level: int, text: str) -> bool:
        url = self._check_url(settings.dingtalk_webhook)
        if not url:
            return False
        return self._is_ok(self._post(url, json={"msgtype": "markdown",
                                          "markdown": {"title": "LabSentinel告警", "text": text}}))


class FeishuChannel(_HttpPushChannel):
    """飞书群机器人：POST open.feishu.cn/open-apis/bot/v2/hook/xxx（text 消息）。"""
    name = "feishu"
    allowed_hosts = FEISHU_ALLOWED_HOSTS

    def configured(self) -> bool:
        return bool((settings.feishu_webhook or "").strip())

    def send(self, level: int, text: str) -> bool:
        url = self._check_url(settings.feishu_webhook)
        if not url:
            return False
        return self._is_ok(self._post(url, json={"msg_type": "text", "content": {"text": text}}))


class ServerChanChannel(_HttpPushChannel):
    """Server酱（推送到微信）：POST sctapi.ftqq.com/<SENDKEY>.send（表单 title+desp）。"""
    name = "serverchan"
    allowed_hosts = SERVERCHAN_ALLOWED_HOSTS

    def configured(self) -> bool:
        return bool((settings.serverchan_sendkey or "").strip())

    def send(self, level: int, text: str) -> bool:
        key = (settings.serverchan_sendkey or "").strip()
        url = self._check_url(f"https://sctapi.ftqq.com/{quote(key, safe='')}.send" if key else None)
        if not url:
            return False
        return self._is_ok(self._post(url, data={"title": alert_title(level), "desp": text}))


class PushPlusChannel(_HttpPushChannel):
    """PushPlus（推送到微信）：POST www.pushplus.plus/send（JSON token+content）。"""
    name = "pushplus"
    allowed_hosts = PUSHPLUS_ALLOWED_HOSTS

    def configured(self) -> bool:
        return bool((settings.pushplus_token or "").strip())

    def send(self, level: int, text: str) -> bool:
        token = (settings.pushplus_token or "").strip()
        if not token:
            return False
        url = self._check_url("https://www.pushplus.plus/send")
        if not url:
            return False
        return self._is_ok(self._post(url, json={"token": token, "title": alert_title(level),
                                          "content": text, "template": "txt"}))


class QmsgChannel(_HttpPushChannel):
    """Qmsg酱（推送到 QQ）：POST qmsg.zendee.cn/send/<KEY>（表单 qq+msg）。"""
    name = "qmsg"
    allowed_hosts = QMSG_ALLOWED_HOSTS

    def configured(self) -> bool:
        return bool((settings.qmsg_key or "").strip() and (settings.qmsg_qq or "").strip())

    def send(self, level: int, text: str) -> bool:
        key = (settings.qmsg_key or "").strip()
        qq = (settings.qmsg_qq or "").strip()
        if not (key and qq):
            return False
        url = self._check_url(f"https://qmsg.zendee.cn/send/{quote(key, safe='')}")
        if not url:
            return False
        return self._is_ok(self._post(url, data={"qq": qq, "msg": text}))


class EmailChannel:
    name = "email"

    def configured(self) -> bool:
        return bool(settings.smtp_host and settings.alert_email_to)

    def send(self, level: int, text: str) -> bool:
        if not (settings.smtp_host and settings.alert_email_to):
            return False
        msg = MIMEText(text, "plain", "utf-8")
        msg["Subject"] = f"[LabSentinel] 级别{level}告警"
        msg["From"] = settings.smtp_user or ""
        msg["To"] = settings.alert_email_to
        # T-210（F-08）：LS_SMTP_SECURITY=ssl（默认 465 兼容现状）|starttls（587）|none（仅私网）
        security = (settings.smtp_security or "ssl").lower()
        try:
            if security == "ssl":
                with smtplib.SMTP_SSL(settings.smtp_host, settings.smtp_port, timeout=10) as s:
                    if settings.smtp_user:
                        s.login(settings.smtp_user, settings.smtp_pass or "")
                    s.sendmail(msg["From"], [settings.alert_email_to], msg.as_string())
            elif security == "starttls":
                with smtplib.SMTP(settings.smtp_host, settings.smtp_port, timeout=10) as s:
                    s.ehlo()
                    s.starttls(context=ssl.create_default_context())
                    if settings.smtp_user:
                        s.login(settings.smtp_user, settings.smtp_pass or "")
                    s.sendmail(msg["From"], [settings.alert_email_to], msg.as_string())
            elif security == "none":
                import ipaddress as _ip
                host_ip = _ip.ip_address(socket.gethostbyname(settings.smtp_host))
                if not (host_ip.is_private or host_ip.is_loopback):
                    print("[email] LS_SMTP_SECURITY=none 仅允许私网 SMTP（明文口令风险）")
                    return False
                with smtplib.SMTP(settings.smtp_host, settings.smtp_port, timeout=10) as s:
                    if settings.smtp_user:
                        s.login(settings.smtp_user, settings.smtp_pass or "")
                    s.sendmail(msg["From"], [settings.alert_email_to], msg.as_string())
            else:
                print(f"[email] 未知 LS_SMTP_SECURITY: {security}")
                return False
            return True
        except Exception as e:  # noqa: BLE001
            print(f"[email] send failed: {e}")
            return False


def match_silence(silences: list[dict], event: dict, now: datetime | None = None) -> dict | None:
    """静默匹配（任务81，纯函数）：命中返回该静默记录，未命中 None。

    scope 规则：global 全匹配；camera 匹配 str(event.camera_id)；level 匹配 str(event.level)；
    type 匹配 event.type。until_ts <= now 的已过期静默不命中。
    """
    now = now or datetime.now()
    for s in silences:
        try:
            if datetime.fromisoformat(s["until_ts"]) <= now:
                continue  # 已过期
        except (ValueError, TypeError, KeyError):
            continue
        scope, target = s.get("scope"), s.get("target")
        if scope == "global":
            return s
        if scope == "camera" and target == str(event.get("camera_id")):
            return s
        if scope == "level" and target == str(event.get("level")):
            return s
        if scope == "type" and target == event.get("type"):
            return s
    return None


class GenericWebhookChannel(_HttpPushChannel):
    """通用 Webhook（任务67）：推给任意自建服务（n8n/自研平台/局域网网关）。

    与厂商渠道的差异：
    - URL 放行独立逻辑：默认仅 https；http 需 webhook_allow_http=true；拒绝带 user:pass@ 的 URL；
      不受域名白名单限制（自建服务域名不可枚举）；
    - 成功判定=HTTP 2xx（不解析业务码）；
    - 可选 HMAC-SHA256 签名：对原始请求体字节计算，X-LS-Signature: sha256=<hex>。
    """
    name = "webhook"

    def configured(self) -> bool:
        return bool((settings.webhook_url or "").strip())

    def _check_url(self, url: str | None) -> str | None:
        if not url:
            return None
        p = urlparse(url)
        if p.scheme not in ("https", "http") or not p.hostname:
            print(f"[{self.name}] URL 被拒绝（仅允许 http/https）: {redact_url(url)}")
            return None
        if p.username or p.password:
            print(f"[{self.name}] URL 被拒绝（不允许携带 user:pass@ 凭证）")
            return None
        if p.scheme == "http" and not settings.webhook_allow_http:
            print(f"[{self.name}] http 被拒绝（webhook_allow_http=false，仅允许 https）")
            return None
        return url

    @staticmethod
    def _is_ok(resp) -> bool:
        return resp is not None and resp.status_code // 100 == 2

    def send(self, level: int, text: str, event: dict | None = None) -> bool:
        url = self._check_url((settings.webhook_url or "").strip())
        if not url:
            return False
        payload = {"version": 1, "title": alert_title(level), "text": text,
                   "level": level, "ts": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
                   "source": "labsentinel"}
        if event:
            payload.update({("event_id" if k == "id" else k): event.get(k) for k in
                            ("id", "camera_id", "type", "detail") if event.get(k) is not None})
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers = {"Content-Type": "application/json",
                   "X-LS-Event": "alert",
                   "X-LS-Timestamp": str(int(time.time()))}
        secret = (settings.webhook_secret or "").strip()
        if secret:
            import hashlib
            import hmac as hmac_mod
            sig = hmac_mod.new(secret.encode("utf-8"), body, hashlib.sha256).hexdigest()
            headers["X-LS-Signature"] = f"sha256={sig}"
        try:
            if self._transport is not None:  # 测试路径
                with httpx.Client(transport=self._transport, follow_redirects=False) as client:
                    resp = client.post(url, content=body, headers=headers,
                                       timeout=settings.webhook_timeout_s)
            else:
                resp = httpx.post(url, content=body, headers=headers,
                                  timeout=settings.webhook_timeout_s, follow_redirects=False)
        except Exception as e:  # noqa: BLE001
            print(f"[{self.name}] send failed: {e}")
            return False
        return self._is_ok(resp)


def redact_url(url: str) -> str:
    from urllib.parse import urlunparse
    p = urlparse(url)
    if p.username or p.password:
        host = p.hostname or ""
        netloc = f"***:***@{host}" + (f":{p.port}" if p.port else "")
        return urlunparse(p._replace(netloc=netloc))
    return url


# 重试退避（秒）：三次指数退避 1s/4s/15s（任务13）
RETRY_BACKOFF = (1.0, 4.0, 15.0)

_KNOWN_CHANNELS = ("wecom", "dingtalk", "feishu", "serverchan", "pushplus",
                   "qmsg", "email", "webhook")


def alert_filters() -> dict:
    """解析 LS_ALERT_FILTERS（任务86）：{渠道名: {min_level:1-3, types:[..]}}。

    非法 JSON/未知渠道/非法级别 → WARNING 并忽略该条（不崩）；默认 {} = 不过滤。
    """
    raw = (settings.alert_filters or "").strip()
    if not raw:
        return {}
    try:
        data = json.loads(raw)
        if not isinstance(data, dict):
            raise ValueError("顶层必须是对象")
    except ValueError as e:
        print(f"[alert-filters] 配置非法（忽略全部）: {e}")
        return {}
    out: dict = {}
    for name, f in data.items():
        if name not in _KNOWN_CHANNELS:
            print(f"[alert-filters] 未知渠道 {name!r}（忽略该条）")
            continue
        if not isinstance(f, dict):
            print(f"[alert-filters] {name} 配置必须是对象（忽略该条）")
            continue
        entry: dict = {}
        ml = f.get("min_level")
        if ml is not None:
            if isinstance(ml, int) and 1 <= ml <= 3:
                entry["min_level"] = ml
            else:
                print(f"[alert-filters] {name} min_level 非法（忽略该字段）: {ml}")
        types = f.get("types")
        if types is not None:
            if isinstance(types, list) and all(isinstance(x, str) for x in types):
                entry["types"] = types
            else:
                print(f"[alert-filters] {name} types 必须为字符串数组（忽略该字段）")
        if entry:
            out[name] = entry
    return out


def channel_filtered(name: str, event_row: dict, filters: dict | None = None) -> str | None:
    """按渠道过滤（任务86，纯函数）：返回原因文本或 None（不过滤）。

    min_level：event.level < min_level → 过滤；types 非空且 event.type 不在 → 过滤。
    """
    f = (filters or {}).get(name)
    if not f:
        return None
    ml = f.get("min_level")
    if ml is not None and event_row.get("level", 0) < ml:
        return f"L{event_row.get('level')} < min_level {ml}"
    types = f.get("types")
    if types and event_row.get("type") not in types:
        return f"type {event_row.get('type')} 不在白名单"
    return None


def _send_accepts_event(ch) -> bool:
    """渠道 send 是否支持 event 关键字参数（任务67：只有通用 Webhook 支持）。"""
    import inspect
    try:
        return "event" in inspect.signature(ch.send).parameters
    except (TypeError, ValueError):
        return False


class AlertDispatcher:
    """渠道派发器：投递记录 + 三次指数退避重试 + 死信。

    - 外部渠道在线程池异步执行（不阻塞分析 worker；沿用任务09异步化）
    - 每渠道一条 deliveries 记录：pending → ok / failed(死信)；失败按 RETRY_BACKOFF 重试
    - WebSocket 推送保持同步（hub 内部已是线程安全投递）
    - store 由 main.lifespan 注入（测试可不注入，仅退化为无记录发送）
    """

    def __init__(self, store=None):
        self.hub = WebSocketHub()
        self.store = store  # Store | None
        # 应用内 WebSocket 之外的全部外部渠道（各自未配置时静默跳过）
        self.channels = [WeComChannel(), DingTalkChannel(), FeishuChannel(),
                         ServerChanChannel(), PushPlusChannel(), QmsgChannel(),
                         EmailChannel(), GenericWebhookChannel()]  # 任务67：+通用 Webhook
        self._pool = concurrent.futures.ThreadPoolExecutor(max_workers=4,
                                                           thread_name_prefix="alert")
        self._closing = threading.Event()  # 108：服务退出时打断退避等待，避免拖住进程
        # T-305（PF-06/F-11）：每渠道滑动窗口限流（默认 20 条/分钟，企微/钉钉官方口径）；
        # 超限不丢弃——累计 pending，窗口放行后的首条文本附摘要
        self._rate: dict[str, deque] = {}
        self._rate_pending: dict[str, int] = {}

    def shutdown(self, wait: bool = False, timeout: float = 3.0):
        """停止接收新投递并尽量取消排队任务；最多等 timeout 秒（108）。

        ThreadPoolExecutor 的线程无法强制杀死；这里用 _closing 打断退避 sleep，
        让运行中的重试在 ~0s 内放弃，正常退出不被 1s/4s/15s 退避拖住。
        """
        self._closing.set()
        try:
            self._pool.shutdown(wait=False, cancel_futures=True)
        except TypeError:  # Python < 3.9（本项目 3.12，仅兜底）
            self._pool.shutdown(wait=False)
        if wait:
            deadline = time.monotonic() + max(0.0, timeout)
            for t in list(threading.enumerate()):
                if not t.name.startswith("alert") or t is threading.current_thread():
                    continue
                left = deadline - time.monotonic()
                if left <= 0:
                    break
                t.join(timeout=left)

    def reset(self):
        """重新启用派发（测试/重启场景：同一进程多次 lifespan，shutdown 后池已关闭）。"""
        if self._closing.is_set():
            self._closing.clear()
            self._pool = concurrent.futures.ThreadPoolExecutor(
                max_workers=2, thread_name_prefix="alert")

    def bind_store(self, store):
        self.store = store

    def dispatch(self, event_row: dict):
        if self._closing.is_set():
            return  # 服务退出中：不再排队新投递
        self.hub.broadcast_sync(event_row)
        try:  # 任务125：MQTT 事件总线（独立于告警链：不受静默/渠道过滤影响；失败吞掉）
            # T-208（B-14）：派发前行里 snapshot 已被 pop（只回布尔）——has_snapshot
            # 由 _dispatch_row 写入；camera_name 走 ConfigCache（避免每事件查库）
            from . import configcache, mqtt_bridge
            zones = []
            if self.store is not None:
                try:  # T-710：事件关联的区域名（detail 含区域名的反查）
                    zs = configcache.zones(self.store, event_row.get("camera_id", 0))
                    detail = event_row.get("detail") or ""
                    zones = [z["name"] for z in zs if z["name"] and z["name"] in detail]
                except Exception:  # noqa: BLE001
                    zones = []
            mqtt_bridge.publish_event(event_row,
                                      camera_name=configcache.camera_name(self.store, event_row.get("camera_id", 0))
                                      if self.store is not None else None,
                                      zones=zones)
        except Exception:  # noqa: BLE001
            pass
        text = render_alert(event_row)
        event_id = event_row.get("id", 0)
        # 任务81：静默命中 → 不建投递行、不重试、不发任何外部渠道；事件/WS 照常
        if self.store is not None:
            sil = match_silence(self.store.silences(active_only=True), event_row)
            if sil is not None:
                print(f"[alert] event#{event_id} 命中静默 #{sil['id']}（{sil['scope']}"
                      f"{':' + str(sil['target']) if sil.get('target') else ''}，"
                      f"至 {sil['until_ts']}）——外部渠道不推送（事件已入库/WS 照常）")
                return
        filters = alert_filters()  # 任务86：每渠道级别/类型过滤（默认 {} 不过滤）
        for ch in self.channels:
            cfg = getattr(ch, "configured", None)  # 测试替身可能没有该方法，默认视为已配置
            if callable(cfg) and not cfg():
                continue  # 未配置渠道整条跳过：不排队、不建投递记录、不产假死信
            ch_name = getattr(ch, "name", type(ch).__name__)
            reason = channel_filtered(ch_name, event_row, filters)
            if reason:
                print(f"[alert] event#{event_id} 渠道 {ch_name} 被过滤（{reason}）——"
                      "不建投递行（事件已入库/WS 照常）")
                continue
            # 任务67：send 支持 event 参数的渠道（通用 Webhook）附带完整事件行，其余渠道签名不变
            extra = event_row if _send_accepts_event(ch) else None
            if self._rate_gate(ch_name):
                summary = self._take_pending(ch_name)
                send_text = text + (f"\n（上一窗口合并 {summary} 条）" if summary else "")
                self._pool.submit(self._deliver_with_retry, ch, event_id,
                                  event_row["level"], send_text, extra)
            else:
                self._pend(ch_name)  # T-305：风暴时限流合并，不丢弃不刷屏

    def dispatch_text(self, event_row: dict, text: str, kind: str = "escalation") -> bool:
        """T-207（B-13）：外部文本（升级提醒等）走完整派发链——静默/渠道过滤/投递记录/
        重试/死信全部复用；返回是否全部渠道接受提交（不等待发送结果）。"""
        if self._closing.is_set() or self.store is None:
            return False
        event_id = event_row.get("id", 0)
        sil = match_silence(self.store.silences(active_only=True), event_row)
        if sil is not None:
            print(f"[alert] event#{event_id} {kind} 命中静默——外部渠道不推送")
            return False
        filters = alert_filters()
        submitted = 0
        for ch in self.channels:
            cfg = getattr(ch, "configured", None)
            if callable(cfg) and not cfg():
                continue
            ch_name = getattr(ch, "name", type(ch).__name__)
            reason = channel_filtered(ch_name, event_row, filters)
            if reason:
                continue
            extra = event_row if _send_accepts_event(ch) else None
            if self._rate_gate(ch_name):
                summary = self._take_pending(ch_name)
                send_text = text + (f"\n（上一窗口合并 {summary} 条）" if summary else "")
                self._pool.submit(self._deliver_with_retry, ch, event_id,
                                  event_row["level"], send_text, extra)
                submitted += 1
            else:
                self._pend(ch_name)
        return submitted > 0

    _RATE_WINDOW_S = 60.0
    _RATE_MAX = 20  # 企微/钉钉官方 20 条/分钟

    def _rate_gate(self, ch_name: str) -> bool:
        """T-305：滑窗闸门——窗口内 <max 放行并记账；超限返回 False（调用方挂 pending）。"""
        dq = self._rate.setdefault(ch_name, deque())
        now = time.monotonic()
        while dq and dq[0] < now - self._RATE_WINDOW_S:
            dq.popleft()
        if len(dq) < self._RATE_MAX:
            dq.append(now)
            return True
        return False

    def _pend(self, ch_name: str) -> None:
        self._rate_pending[ch_name] = self._rate_pending.get(ch_name, 0) + 1

    def _take_pending(self, ch_name: str) -> int:
        n = self._rate_pending.pop(ch_name, 0)
        return n

    def _deliver_with_retry(self, ch, event_id: int, level: int, text: str, event: dict | None = None):
        """单渠道投递：记录 → 重试（指数退避）→ 死信。线程池内执行。"""
        if self._closing.is_set():
            return
        name = getattr(ch, "name", type(ch).__name__)
        did = self.store.add_delivery(event_id, name) if self.store else None
        last_error = None
        attempts = 0
        for i in range(len(RETRY_BACKOFF) + 1):  # 首次 + 3 次重试
            if self._closing.is_set():
                return  # 退出中：不再发/不再等退避
            attempts = i + 1
            try:
                ok = bool(ch.send(level, text, event=event) if event is not None
                          else ch.send(level, text))
                last_error = None if ok else "渠道返回失败（业务码错误或 HTTP 非 2xx）"
            except Exception as e:  # noqa: BLE001
                ok, last_error = False, f"{type(e).__name__}: {e}"
            if ok:
                if did is not None:
                    self.store.update_delivery(did, "ok", attempts, None)
                return
            if i < len(RETRY_BACKOFF):
                if self._closing.wait(RETRY_BACKOFF[i]):  # 108：可被 shutdown 立即打断
                    return
        if did is not None:
            self.store.update_delivery(did, "failed", attempts, last_error)
        print(f"[alert] {name} 投递失败（死信）event#{event_id}: {last_error}")

    def _test_channel(self, ch, text: str) -> dict:
        """单渠道自检（108）：并发执行体，返回稳定结构。"""
        name = getattr(ch, "name", type(ch).__name__)
        did = self.store.add_delivery(0, name) if self.store else None
        try:
            ok = bool(ch.send(1, text))
            err = None if ok else "发送失败"
        except Exception as e:  # noqa: BLE001
            ok, err = False, f"{type(e).__name__}: {e}"
        if did is not None:
            self.store.update_delivery(did, "ok" if ok else "failed", 1, err)
        return {"channel": name, "ok": ok, "error": err}

    def send_test(self, channel: str | None = None) -> list[dict]:
        """渠道健康自检（任务13C/108/140）：默认测全部；channel 指定时只测该渠道。

        每渠道自身超时仍为 5s（邮件连接 10s）；future 等待上限 6s，避免串行 8 渠道拖到 ~50s。
        """
        text = render_alert({
            "ts": datetime.now().isoformat(timespec="seconds"), "camera_id": 0,
            "type": "alert_test", "level": 1, "track_id": 0,
            "detail": "渠道自检消息，收到即表示该渠道配置正常。"})
        channels = self.channels
        if channel:
            channels = [ch for ch in self.channels
                        if getattr(ch, "name", type(ch).__name__) == channel]
            if not channels:
                return [{"channel": channel, "ok": False, "error": "未知渠道"}]
        results: list[dict | None] = [None] * len(channels)
        todo: list[tuple[int, object]] = []
        for i, ch in enumerate(channels):
            name = getattr(ch, "name", type(ch).__name__)
            cfg = getattr(ch, "configured", None)
            if callable(cfg) and not cfg():  # 未配置：不提交、不落投递记录
                results[i] = {"channel": name, "ok": False, "error": "未配置"}
                continue
            todo.append((i, ch))
        if todo:
            ex = concurrent.futures.ThreadPoolExecutor(
                max_workers=min(8, len(todo)), thread_name_prefix="alert-test")
            futs = {ex.submit(self._test_channel, ch, text): i for i, ch in todo}
            try:
                for fut, idx in futs.items():
                    try:
                        results[idx] = fut.result(timeout=6.0)
                    except concurrent.futures.TimeoutError:
                        name = getattr(channels[idx], "name",
                                       type(channels[idx]).__name__)
                        results[idx] = {"channel": name, "ok": False,
                                        "error": "自检超时（>6s）"}
                    except Exception as e:  # noqa: BLE001
                        name = getattr(channels[idx], "name",
                                       type(channels[idx]).__name__)
                        results[idx] = {"channel": name, "ok": False,
                                        "error": f"{type(e).__name__}: {e}"}
            finally:
                ex.shutdown(wait=False, cancel_futures=True)
        return [r for r in results if r is not None]


dispatcher = AlertDispatcher()
