"""脱敏（任务91）：collect_diagnostics 与 API config/summary 共用，防两套规则漂移。

T-109（S-08）：补"路径内令牌"——飞书 hook/<token>、Server酱 <SENDKEY>.send、
企微/钉钉 webhook 路径段、通用 webhook 路径随机串（≥16 位）一并掩码。
"""
import re

# 键名含这些子串 → 值全掩码
SENSITIVE = re.compile(r"TOKEN|PASSWORD|SECRET|KEY|PASS\b|CREDENTIAL|WEBHOOK|SENDKEY", re.IGNORECASE)
# URL 凭证 user:pass@ → ***:***@
URL_CRED = re.compile(r"(?i)(\w[\w+.-]*://)([^/@\s:]+):([^@\s]+)@")
# URL query token=… → token=***
URL_QUERY_TOKEN = re.compile(r"(?i)([?&](?:token|key|access_token|sign|signature|secret|sendkey)=)[^&\s]+")
# 路径内令牌段：/hook|send|robot/ 后的随机串（飞书/钉钉形态）
PATH_SECRET = re.compile(r"(?i)(/(?:hook|send|robot|bot)/)([A-Za-z0-9_\-]{12,})")
# 路径末段 ≥16 位随机串（通用 webhook 令牌；排除常见静态词与文件扩展名段）
LONG_TOKEN_SEGMENT = re.compile(r"(?<=/)(?![vV]\d)(?!\d)[A-Za-z0-9_\-]{16,}(?=/|[.?]|$)")


def redact_value(v: str) -> str:
    """值级脱敏：URL 凭证、query token、路径内令牌段全部掩码。"""
    s = URL_CRED.sub(r"\1***:***@", str(v))
    s = URL_QUERY_TOKEN.sub(r"\1***", s)
    s = PATH_SECRET.sub(r"\1***", s)
    s = LONG_TOKEN_SEGMENT.sub("***", s)
    return s


def redact_line(key: str, value: str) -> str:
    """行级脱敏：敏感键名 → ***；普通键仅脱 URL 凭证与路径令牌。"""
    if SENSITIVE.search(key):
        return f"{key}=***"
    return f"{key}={redact_value(value)}"
