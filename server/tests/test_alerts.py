"""单元测试：告警文案模板 + 全部 HTTP 推送渠道（白名单/静默跳过/mock 网络）。

网络请求一律用 httpx.MockTransport 注入，不发真实请求。运行: pytest tests/ -q
"""
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import httpx  # noqa: E402
import pytest  # noqa: E402

from app.alerts import (DingTalkChannel, FeishuChannel, PushPlusChannel,  # noqa: E402
                        QmsgChannel, ServerChanChannel, WeComChannel, render_alert)
from app.config import settings  # noqa: E402


def _row(level=2, type_="enter_no_ppe", detail="进入安全区「A」未穿实验服"):
    return {"id": 1, "ts": "2026-10-25T12:30:00", "camera_id": 1, "type": type_,
            "level": level, "track_id": 7, "detail": detail}


# ---- 文案模板 ----

def test_render_alert_levels():
    r1 = render_alert(_row(level=1, type_="exit", detail="离开安全区「A」"))
    assert "ℹ️" in r1 and "L1" in r1 and "离开安全区" in r1 and "离开安全区「A」" in r1
    r2 = render_alert(_row(level=2))
    assert "⚠️" in r2 and "进入安全区未穿实验服" in r2 and "2026-10-25T12:30:00" in r2
    assert "track 7" in r2
    # occupancy_over 无具体跟踪目标 → 隐藏 track 行（任务09）
    r3 = render_alert(_row(level=3, type_="occupancy_over", detail="在场人数 5 超过阈值 3"))
    assert "🔴" in r3 and "在场人数超限" in r3 and "#1" in r3 and "track" not in r3
    # 未知类型回退原值、未知级别回退默认图标
    r4 = render_alert(_row(level=9, type_="unknown_type"))
    assert "unknown_type" in r4 and "ℹ️" in r4


# ---- 渠道通用参数化测试 ----
# (渠道类, 配置项→值, 期望请求 URL 前缀, 期望出现在请求体里的内容片段)
CHANNELS = [
    (WeComChannel, {"wecom_webhook": "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=k"},
     "https://qyapi.weixin.qq.com/", ["markdown", "LabSentinel"]),
    (DingTalkChannel, {"dingtalk_webhook": "https://oapi.dingtalk.com/robot/send?access_token=t"},
     "https://oapi.dingtalk.com/", ["msgtype", "LabSentinel告警"]),
    (FeishuChannel, {"feishu_webhook": "https://open.feishu.cn/open-apis/bot/v2/hook/x"},
     "https://open.feishu.cn/", ["msg_type", "text"]),
    (ServerChanChannel, {"serverchan_sendkey": "SCT123"},
     "https://sctapi.ftqq.com/SCT123.send", ["title", "desp"]),
    (PushPlusChannel, {"pushplus_token": "pp123"},
     "https://www.pushplus.plus/send", ["token", "pp123"]),
    (QmsgChannel, {"qmsg_key": "qm123", "qmsg_qq": "10000"},
     "https://qmsg.zendee.cn/send/qm123", ["qq", "10000"]),
]


@pytest.mark.parametrize("cls,cfg,url_prefix,body_parts", CHANNELS)
def test_channel_send_ok(monkeypatch, cls, cfg, url_prefix, body_parts):
    for k, v in cfg.items():
        monkeypatch.setattr(settings, k, v)
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        raw = request.content.decode("utf-8")
        if "json" in request.headers.get("content-type", ""):
            captured["body"] = json.loads(raw)
        else:  # 表单型渠道（Server酱/Qmsg）
            from urllib.parse import parse_qsl
            captured["body"] = dict(parse_qsl(raw))
        return httpx.Response(200, json={"errcode": 0})

    ch = cls(transport=httpx.MockTransport(handler))
    ch.send(2, "⚠️ 测试文案")
    assert captured["url"].startswith(url_prefix)
    raw = json.dumps(captured["body"], ensure_ascii=False)
    for part in body_parts:
        assert part in raw
    assert "⚠️ 测试文案" in raw


URL_CHANNELS = [c for c in CHANNELS if c[0] in (WeComChannel, DingTalkChannel, FeishuChannel)]
KEY_CHANNELS = [c for c in CHANNELS if c[0] in (ServerChanChannel, PushPlusChannel, QmsgChannel)]


@pytest.mark.parametrize("cls,cfg,_,__", URL_CHANNELS)
def test_url_channel_whitelist_rejects(monkeypatch, cls, cfg, _, __):
    """URL 型渠道：白名单外/非 https/空配置一律不发（零请求）。"""
    calls = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append(str(request.url))
        return httpx.Response(200)

    ch = cls(transport=httpx.MockTransport(handler))
    attr = list(cfg)[0]
    host = next(iter(cls.allowed_hosts))
    for bad in (f"http://{host}/x",                      # 非 https
                f"https://{host}.evil.com/x",            # 仿冒后缀
                "https://evil.com/x",                    # 其他域名
                "", None):                               # 空 = 不启用
        monkeypatch.setattr(settings, attr, bad)
        ch.send(2, "x")
    assert calls == []


@pytest.mark.parametrize("cls,cfg,_,__", KEY_CHANNELS)
def test_key_channel_empty_skip(monkeypatch, cls, cfg, _, __):
    """key 型渠道：key/QQ 未配置（空或 None）时不发请求。"""
    calls = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append(str(request.url))
        return httpx.Response(200)

    ch = cls(transport=httpx.MockTransport(handler))
    for attr in cfg:
        for empty in ("", None):
            monkeypatch.setattr(settings, attr, empty)
            ch.send(2, "x")
    # Qmsg：key 在但缺 QQ 号也跳过
    if cls is QmsgChannel:
        monkeypatch.setattr(settings, "qmsg_key", cfg["qmsg_key"])
        monkeypatch.setattr(settings, "qmsg_qq", "")
        ch.send(2, "x")
    assert calls == []


# ---- 任务09：告警通道打磨 ----

def test_render_alert_occupancy_no_track():
    r = render_alert(_row(level=2, type_="occupancy_over", detail="在场人数 5 超过阈值 3"))
    assert "track" not in r and "在场人数超限" in r and "#1" in r


def test_dispatch_nonblocking():
    """慢渠道（SMTP 5s）不得阻塞 dispatch：异步提交后立即返回。"""
    import time as _t
    from app.alerts import AlertDispatcher

    class SlowChannel:
        name = "slow"

        def send(self, level, text):
            _t.sleep(5)

    d = AlertDispatcher()
    d.channels = [SlowChannel()]
    t0 = _t.monotonic()
    d.dispatch(_row())
    assert _t.monotonic() - t0 < 1.0  # 远小于渠道的 5s 阻塞
    d._pool.shutdown(wait=False, cancel_futures=True)


@pytest.mark.parametrize("cls,cfg,pathpart", [
    (ServerChanChannel, {"serverchan_sendkey": "a/b c"}, "a%2Fb%20c.send"),
    (QmsgChannel, {"qmsg_key": "k/x", "qmsg_qq": "10000"}, "k%2Fx"),
])
def test_key_urlencoded(monkeypatch, cls, cfg, pathpart):
    for k, v in cfg.items():
        monkeypatch.setattr(settings, k, v)
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["path"] = request.url.raw_path.decode()  # 原始（编码后）路径
        return httpx.Response(200)

    cls(transport=httpx.MockTransport(handler)).send(2, "x")
    assert pathpart in captured["path"]
