"""单元测试：任务78 camera_soak 增强（时长/ffprobe 解析/汇总判定/停滞边界）。纯函数，无真实进程。"""
import importlib.util
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "tools"))

import pytest  # noqa: E402

import camera_soak as cs  # noqa: E402


def test_parse_duration():
    assert cs.parse_duration("24h") == 86400
    assert cs.parse_duration("90m") == 5400
    assert cs.parse_duration("3600") == 3600
    assert cs.parse_duration("30s") == 30
    with pytest.raises(ValueError):
        cs.parse_duration("abc")


def test_parse_ffprobe_json():
    h264 = json.dumps({"streams": [{"codec_name": "h264", "width": 1280, "height": 720,
                                    "avg_frame_rate": "25/1"}]})
    d = cs.parse_ffprobe_json(h264)
    assert d == {"codec": "h264", "width": 1280, "height": 720, "fps": 25.0}
    h265 = cs.parse_ffprobe_json(json.dumps(
        {"streams": [{"codec_name": "hevc", "width": 3840, "height": 2160,
                      "avg_frame_rate": "0/0"}]}))
    assert h265["codec"] == "hevc" and h265["fps"] == 0.0  # 0/0 → 0 不崩
    assert cs.parse_ffprobe_json("not json") is None
    assert cs.parse_ffprobe_json('{"streams": []}') is None


def _row(cam_id, status="running", fps=25.0, age=0.0, recon=0, err="", lat=20.0, ev=10):
    return {"ts": "2026-10-31 00:00:00", "camera_id": cam_id, "status": status, "fps": fps,
            "frame_age_s": age, "reconnect_count": recon, "last_error": err,
            "events_total": ev, "api_latency_ms": lat, "cpu_percent": 30.0,
            "rss_mb": 2000.0, "disk_free_gb": 100.0}


def test_summarize_pass():
    rows = [_row(1), _row(1), _row(1)]
    s = cs.summarize(rows, 600)
    assert s["verdict"] == "PASS"
    assert s["api_rate_pct"] == 100.0
    assert s["cameras"]["1"]["running_rate_pct"] == 100.0
    assert s["events_growth"] == 0


def test_summarize_fail_on_error_and_rate():
    rows = [_row(1, status="error", err="打开失败"), _row(""), _row(1)]
    s = cs.summarize(rows, 600)
    assert s["verdict"] == "FAIL"  # running 率 <99% + 错误
    assert any(c["errors"] for c in s["cameras"].values())


def test_summarize_warn_on_reconnect_and_stall():
    rows = [_row(1, recon=15), _row(1, age=200.0)]  # 停滞 > stall*3
    s = cs.summarize(rows, 600, stall_s=60)
    assert s["verdict"] == "WARN"
    assert s["cameras"]["1"]["stall_samples"] == 1
    # 边界：age 恰好 == stall 不算停滞（> 才算）
    rows2 = [_row(1, age=60.0)]
    assert cs.summarize(rows2, 600)["cameras"]["1"]["stall_samples"] == 0


def test_summarize_events_growth_and_latency_pct():
    rows = [_row(1, ev=100, lat=10.0), _row(1, ev=150, lat=30.0), _row(1, ev=200, lat=100.0)]
    s = cs.summarize(rows, 600)
    assert s["events_growth"] == 100
    assert s["api_latency_ms"]["p50"] == 30.0 and s["api_latency_ms"]["max"] == 100.0


def test_build_row_fields():
    r = cs.build_row("ts", {"id": 3, "status": "running", "fps": 25.0, "frame_age_s": 0.1,
                            "reconnect_count": 1, "last_error": None},
                     12.34, 55, "42.0", "", 88.8)
    assert list(r) == cs.CSV_FIELDS
    assert r["last_error"] == "" and r["events_total"] == 55


def test_preflight_no_server(tmp_path, capsys):
    """服务不可达：清晰提示不崩，FAIL 退出码 2。"""
    old = sys.argv
    sys.argv = ["camera_soak.py", "--preflight", "--base-url", "http://127.0.0.1:59999"]
    try:
        rc = cs.main()
    finally:
        sys.argv = old
    out = capsys.readouterr().out
    assert rc == 2 and "服务可达" in out and "FAIL" in out
