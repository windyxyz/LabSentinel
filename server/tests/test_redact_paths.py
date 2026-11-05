"""T-109（S-08）回归：路径内令牌脱敏（飞书/企微/钉钉/Server酱/通用 webhook）。"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from app.redact import redact_line, redact_value  # noqa: E402


def test_vendor_webhook_paths_masked():
    cases = {
        # 飞书
        "https://open.feishu.cn/open-apis/bot/v2/hook/AbCdEf123456-GhIjKl-789":
            "https://open.feishu.cn/open-apis/bot/v2/hook/***",
        # 企微（query 参数名保留、值掩码）
        "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=WxKey1234567890abcdef":
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=***",
        # 钉钉
        "https://oapi.dingtalk.com/robot/send?access_token=DTok1234567890abcdef":
            "https://oapi.dingtalk.com/robot/send?access_token=***",
        # Server酱（路径段 SENDKEY.send）
        "https://sctapi.ftqq.com/SCT1234567890abcdefghijkl.send":
            "https://sctapi.ftqq.com/***.send",
        # 通用 webhook 路径随机串
        "https://example.com/alert/wh_a1b2c3d4e5f6g7h8i9j0/receive":
            "https://example.com/alert/***/receive",
    }
    for raw, want in cases.items():
        assert redact_value(raw) == want, raw


def test_sensitive_key_names_fully_masked():
    for key in ("LS_WECOM_WEBHOOK", "LS_FEISHU_WEBHOOK", "LS_SMTP_PASS", "LS_SENDKEY"):
        assert redact_line(key, "https://whatever/hook/secret123") == f"{key}=***"


def test_normal_values_untouched():
    assert redact_value("普通中文文本") == "普通中文文本"
    assert redact_value("v1.2.3") == "v1.2.3"
    assert redact_value("2026-11-05T12:00:00") == "2026-11-05T12:00:00"
