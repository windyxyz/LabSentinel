"""脱敏（任务91）：collect_diagnostics 与 API config/summary 共用，防两套规则漂移。"""
import re

# 键名含这些子串 → 值全掩码
SENSITIVE = re.compile(r"TOKEN|PASSWORD|SECRET|KEY|PASS\b|CREDENTIAL", re.IGNORECASE)
# URL 凭证 user:pass@ → ***:***@
URL_CRED = re.compile(r"(?i)(\w[\w+.-]*://)([^/@\s:]+):([^@\s]+)@")
# URL query token=… → token=***
URL_QUERY_TOKEN = re.compile(r"(?i)([?&](?:token|key|access_token|sign|signature|secret)=)[^&\s]+")


def redact_value(v: str) -> str:
    """值级脱敏：URL 凭证与 query token 抹除。"""
    s = URL_CRED.sub(r"\1***:***@", str(v))
    s = URL_QUERY_TOKEN.sub(r"\1***", s)
    return s


def redact_line(key: str, value: str) -> str:
    """行级脱敏：敏感键名 → ***；普通键仅脱 URL 凭证。"""
    if SENSITIVE.search(key):
        return f"{key}=***"
    return f"{key}={redact_value(value)}"
