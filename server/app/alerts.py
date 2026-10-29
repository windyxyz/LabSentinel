"""告警通道：应用内 WebSocket + 企业微信 + 钉钉 + 飞书 + 微信(Server酱/PushPlus) + QQ(Qmsg酱) + SMTP 邮件。

所有 HTTP 推送渠道共用 _HttpPushChannel 基类：域名白名单 + 强制 https +
不跟随重定向（防 SSRF），文案统一走 render_alert。
"""
import asyncio
import concurrent.futures
import json
import smtplib
import time
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
    """各推送渠道共用的告警文案模板：级别图标 + 级别 + 类型 + 时间 + 摄像头 + 详情。

    occupancy_over 是无具体跟踪目标的规则事件，不输出 track 行。
    """
    icon = LEVEL_ICON.get(row["level"], "ℹ️")
    type_text = TYPE_TEXT.get(row["type"], row["type"])
    head = (f"{icon} L{row['level']} {type_text} · {row['ts']}\n"
            f"摄像头 #{row['camera_id']}")
    if row["type"] != "occupancy_over":
        head += f" · track {row.get('track_id', 0)}"
    return f"{head}\n{row['detail']}"


def alert_title(level: int) -> str:
    return f"LabSentinel 级别{level}告警"


class WebSocketHub:
    """事件实时推送。worker 线程通过 broadcast_sync 投递。"""

    def __init__(self):
        self._clients: list[WebSocket] = []
        self._loop: asyncio.AbstractEventLoop | None = None

    def bind_loop(self, loop):
        self._loop = loop

    async def connect(self, ws: WebSocket):
        await ws.accept()
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
        dead = []
        for ws in self._clients:
            try:
                await ws.send_text(json.dumps(payload, ensure_ascii=False))
            except Exception:  # noqa: BLE001
                dead.append(ws)
        for ws in dead:
            self.disconnect(ws)


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
        try:
            with smtplib.SMTP_SSL(settings.smtp_host, settings.smtp_port, timeout=10) as s:
                if settings.smtp_user:
                    s.login(settings.smtp_user, settings.smtp_pass or "")
                s.sendmail(msg["From"], [settings.alert_email_to], msg.as_string())
            return True
        except Exception as e:  # noqa: BLE001
            print(f"[email] send failed: {e}")
            return False


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
        self._pool = concurrent.futures.ThreadPoolExecutor(max_workers=2,
                                                           thread_name_prefix="alert")

    def bind_store(self, store):
        self.store = store

    def dispatch(self, event_row: dict):
        self.hub.broadcast_sync(event_row)
        text = render_alert(event_row)
        event_id = event_row.get("id", 0)
        for ch in self.channels:
            cfg = getattr(ch, "configured", None)  # 测试替身可能没有该方法，默认视为已配置
            if callable(cfg) and not cfg():
                continue  # 未配置渠道整条跳过：不排队、不建投递记录、不产假死信
            # 任务67：send 支持 event 参数的渠道（通用 Webhook）附带完整事件行，其余渠道签名不变
            extra = event_row if _send_accepts_event(ch) else None
            self._pool.submit(self._deliver_with_retry, ch, event_id,
                              event_row["level"], text, extra)

    def _deliver_with_retry(self, ch, event_id: int, level: int, text: str, event: dict | None = None):
        """单渠道投递：记录 → 重试（指数退避）→ 死信。线程池内执行。"""
        name = getattr(ch, "name", type(ch).__name__)
        did = self.store.add_delivery(event_id, name) if self.store else None
        last_error = None
        attempts = 0
        for i in range(len(RETRY_BACKOFF) + 1):  # 首次 + 3 次重试
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
                time.sleep(RETRY_BACKOFF[i])  # 1s/4s/15s
        if did is not None:
            self.store.update_delivery(did, "failed", attempts, last_error)
        print(f"[alert] {name} 投递失败（死信）event#{event_id}: {last_error}")

    def send_test(self) -> list[dict]:
        """渠道健康自检（任务13C）：向所有已配置渠道发测试消息，event_id=0 不落 events。"""
        results = []
        for ch in self.channels:
            name = getattr(ch, "name", type(ch).__name__)
            cfg = getattr(ch, "configured", None)
            if callable(cfg) and not cfg():  # 自检结果里明确标注未配置，但不落投递记录
                results.append({"channel": name, "ok": False, "error": "未配置"})
                continue
            did = self.store.add_delivery(0, name) if self.store else None
            try:
                ok = bool(ch.send(1, "【测试】LabSentinel 渠道自检消息，收到即表示该渠道配置正常。"))
                err = None if ok else "发送失败"
            except Exception as e:  # noqa: BLE001
                ok, err = False, f"{type(e).__name__}: {e}"
            if did is not None:
                self.store.update_delivery(did, "ok" if ok else "failed", 1, err)
            results.append({"channel": name, "ok": ok, "error": err})
        return results


dispatcher = AlertDispatcher()
