"""网页端告警渠道配置（任务 140）。

目标：管理员在网页上按指引填写各渠道凭据，保存即生效，不需要改 .env / 重启。
- 配置存本机 SQLite `app_settings`（key=alert.<settings字段名>），随数据库备份；
- API 永不回显密钥/完整 webhook：只返回 `has_*` 或掩码 URL；
- 空值 = 保持原值（避免误清空）；「清除本渠道」会删 DB 覆盖并回退到启动时的 .env 值；
- 保存前校验 URL 域名/scheme、端口、邮箱、QQ 等；失败 400 不写库；
- 与告警发送解耦：alerts.py 每次发送都读 settings，所以 setattr 后立即生效。
"""
import re
from urllib.parse import parse_qs, urlparse, urlunparse

from . import alerts
from .config import settings

PREFIX = "alert."

# channel -> {API 字段: Settings 字段}
FIELDS: dict[str, dict[str, str]] = {
    "wecom": {"webhook": "wecom_webhook"},
    "dingtalk": {"webhook": "dingtalk_webhook"},
    "feishu": {"webhook": "feishu_webhook"},
    "serverchan": {"sendkey": "serverchan_sendkey"},
    "pushplus": {"token": "pushplus_token"},
    "qmsg": {"key": "qmsg_key", "qq": "qmsg_qq"},
    "email": {"smtp_host": "smtp_host", "smtp_port": "smtp_port",
              "smtp_user": "smtp_user", "smtp_pass": "smtp_pass",
              "to": "alert_email_to"},
    "webhook": {"url": "webhook_url", "secret": "webhook_secret",
                "allow_http": "webhook_allow_http"},
}

SECRET_FIELDS = {"wecom_webhook", "dingtalk_webhook", "feishu_webhook",
                 "serverchan_sendkey", "pushplus_token", "qmsg_key",
                 "smtp_pass", "webhook_secret"}
URL_FIELDS = {"wecom_webhook", "dingtalk_webhook", "feishu_webhook", "webhook_url"}
BOOL_FIELDS = {"webhook_allow_http"}
INT_FIELDS = {"smtp_port"}
ALL_FIELDS = {f for m in FIELDS.values() for f in m.values()}

# 启动时（.env 已加载）的原始值，用于「清除覆盖」回退
_DEFAULTS = {f: getattr(settings, f) for f in ALL_FIELDS}

_URL_HOSTS = {
    "wecom": alerts.ALLOWED_WEBHOOK_HOSTS,
    "dingtalk": alerts.DINGTALK_ALLOWED_HOSTS,
    "feishu": alerts.FEISHU_ALLOWED_HOSTS,
}
_EMAIL_RE = re.compile(r"^[^@\s]+@[^@\s]+\.[^@\s]+$")


class AlertConfigError(ValueError):
    pass


def _serialize(field: str, value) -> str:
    if field in BOOL_FIELDS:
        return "1" if value else "0"
    text = str(value)
    # T-107（S-06）：敏感字段入库存密文（明文只在内存 settings 中）
    if field in SECRET_FIELDS and text:
        from .secretstore import protect
        return protect(text, settings.data_dir)
    return text


def _deserialize(field: str, raw: str):
    if field in BOOL_FIELDS:
        return raw.strip() in ("1", "true", "True", "yes", "on")
    if field in INT_FIELDS:
        return int(raw)
    if field in SECRET_FIELDS:
        from .secretstore import unprotect
        return unprotect(raw, settings.data_dir)  # 兼容历史明文
    return raw


def _mask_url(url: str) -> str:
    """掩码显示 webhook：保留 scheme/host/路径结构，查询值全 ***，末段长 token 打码。"""
    if not url:
        return ""
    try:
        p = urlparse(url)
    except ValueError:
        return "***"
    parts = [x for x in p.path.split("/") if x]
    if parts and len(parts[-1]) >= 8:
        parts[-1] = "***"
    query = ""
    if p.query:
        try:
            keys = list(parse_qs(p.query, keep_blank_values=True).keys())
        except ValueError:
            keys = []
        query = "&".join(f"{k}=***" for k in keys)
    return urlunparse((p.scheme, p.netloc, "/" + "/".join(parts) if parts else "",
                       "", query, ""))


def _configured(channel: str) -> bool:
    if channel == "wecom":
        return bool((settings.wecom_webhook or "").strip())
    if channel == "dingtalk":
        return bool((settings.dingtalk_webhook or "").strip())
    if channel == "feishu":
        return bool((settings.feishu_webhook or "").strip())
    if channel == "serverchan":
        return bool((settings.serverchan_sendkey or "").strip())
    if channel == "pushplus":
        return bool((settings.pushplus_token or "").strip())
    if channel == "qmsg":
        return bool((settings.qmsg_key or "").strip() and (settings.qmsg_qq or "").strip())
    if channel == "email":
        return bool(settings.smtp_host and settings.alert_email_to)
    if channel == "webhook":
        return bool((settings.webhook_url or "").strip())
    return False


def _value(field: str):
    return getattr(settings, field)


def load(store) -> None:
    """启动时加载 DB 覆盖到 settings（.env 已被 pydantic 读取；DB 优先）。"""
    rows = store.app_settings(PREFIX)
    for key, raw in rows.items():
        field = key[len(PREFIX):]
        if field not in ALL_FIELDS:
            continue
        try:
            setattr(settings, field, _deserialize(field, raw))
        except (TypeError, ValueError):
            print(f"[alert-config] 忽略非法配置 {key}")


def status(store) -> dict:
    db_fields = {k[len(PREFIX):] for k in store.app_settings(PREFIX)}
    out: dict[str, dict] = {}
    for channel, mapping in FIELDS.items():
        item: dict = {"configured": _configured(channel)}
        fields = set(mapping.values())
        item["source"] = ("db" if fields & db_fields
                          else "env" if any(_value(f) not in (None, "", False) for f in fields)
                          else "none")
        for api_name, field in mapping.items():
            val = _value(field)
            if field in URL_FIELDS:
                item[f"has_{api_name}"] = bool(val)
                item[api_name] = _mask_url(str(val or ""))
            elif field in SECRET_FIELDS:
                item[f"has_{api_name}"] = bool(val)
                item[api_name] = ""
            elif field in BOOL_FIELDS:
                item[api_name] = bool(val)
            elif field in INT_FIELDS:
                item[api_name] = int(val) if val is not None else None
            else:
                item[api_name] = val if val is not None else ""
        out[channel] = item
    return {"channels": out,
            "note": "密钥/完整 webhook 不会回显；留空保存=保持原值；清除=回退启动时 .env"}


def _validate_url(channel: str, url: str) -> None:
    p = urlparse(url)
    if p.username or p.password:
        raise AlertConfigError("Webhook URL 不能携带 user:pass@ 凭证")
    if channel == "webhook":
        if p.scheme not in ("https", "http") or not p.hostname:
            raise AlertConfigError("Webhook URL 必须是 http(s) 且含主机")
        return
    allowed = _URL_HOSTS.get(channel)
    if allowed is None:
        return
    if p.scheme != "https" or p.hostname not in allowed:
        raise AlertConfigError(
            f"该渠道只允许 https 且域名为 {', '.join(sorted(allowed))}")


def _check_value(channel: str, api_name: str, field: str, value):
    """返回要写入 settings 的值；非法抛 AlertConfigError。"""
    if field in BOOL_FIELDS:
        if isinstance(value, bool):
            return value
        return str(value).strip().lower() in ("1", "true", "yes", "on")
    if field in INT_FIELDS:
        try:
            n = int(value)
        except (TypeError, ValueError):
            raise AlertConfigError(f"{api_name} 必须是整数")
        if not 1 <= n <= 65535:
            raise AlertConfigError(f"{api_name} 必须在 1-65535")
        return n
    s = str(value).strip()
    if field in URL_FIELDS and s:
        _validate_url(channel, s)
    if field == "alert_email_to" and s and not _EMAIL_RE.match(s):
        raise AlertConfigError("收件邮箱格式不正确")
    if field == "smtp_host" and s and ("://" in s or " " in s):
        raise AlertConfigError("SMTP 服务器只填主机名，如 smtp.qq.com")
    if field == "qmsg_qq" and s and not s.isdigit():
        raise AlertConfigError("Qmsg 接收 QQ 必须是数字")
    return s


def save(store, body: dict) -> dict:
    """校验并保存网页提交的渠道配置；空字符串/None 表示保持原值。"""
    if not isinstance(body, dict):
        raise AlertConfigError("body 必须是对象")
    updates: dict[str, str] = {}
    pending: dict[str, object] = {}
    for channel, values in body.items():
        mapping = FIELDS.get(channel)
        if mapping is None:
            raise AlertConfigError(f"未知渠道: {channel}")
        if not isinstance(values, dict):
            raise AlertConfigError(f"{channel} 配置必须是对象")
        unknown = set(values) - set(mapping)
        if unknown:
            raise AlertConfigError(f"{channel} 含未知字段: {sorted(unknown)}")
        for api_name, raw in values.items():
            if raw is None or (isinstance(raw, str) and not raw.strip()):
                continue  # 留空=保持
            field = mapping[api_name]
            pending[field] = _check_value(channel, api_name, field, raw)
    # 组合校验（以"当前值 + 本次更新"为准）
    def eff(field):
        return pending.get(field, _value(field))
    if eff("webhook_url"):
        url = str(eff("webhook_url")).strip()
        if url.startswith("http://") and not eff("webhook_allow_http"):
            raise AlertConfigError("http:// webhook 需要同时勾选“允许 HTTP（仅内网）”")
        if url:
            _validate_url("webhook", url)
    if eff("smtp_host") or eff("alert_email_to"):
        if not (eff("smtp_host") and eff("alert_email_to")):
            raise AlertConfigError("邮件渠道需要同时填写 SMTP 服务器和收件邮箱")
    if eff("qmsg_key") or eff("qmsg_qq"):
        if not (eff("qmsg_key") and eff("qmsg_qq")):
            raise AlertConfigError("Qmsg 需要同时填写 Key 和接收 QQ")
    if pending:
        store.set_app_settings({PREFIX + f: _serialize(f, v) for f, v in pending.items()})
        for field, value in pending.items():
            setattr(settings, field, value)
    return status(store)


def reset(store, channel: str | None = None) -> dict:
    """清除 DB 覆盖，回退启动时 .env 值；channel=None 清全部。"""
    if channel is not None and channel not in FIELDS:
        raise AlertConfigError(f"未知渠道: {channel}")
    fields = ([f for f in FIELDS[channel].values()] if channel
              else sorted(ALL_FIELDS))
    store.delete_app_settings([PREFIX + f for f in fields])
    for field in fields:
        setattr(settings, field, _DEFAULTS[field])
    return status(store)
