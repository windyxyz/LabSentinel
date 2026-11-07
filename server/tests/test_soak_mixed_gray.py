"""T-327 灰度验收演练脚本纯函数回归（不依赖运行实例）。"""
import os
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import soak_mixed_gray as s  # noqa: E402


def test_soak_summary_pass_fail():
    good = {"cameras": {"1": {"running_rate_pct": 100.0, "fps_avg": 5.0}},
            "samples": 10, "verdict": "PASS"}
    assert s.check_soak_summary(good) == []
    bad = {"cameras": {"1": {"running_rate_pct": 90.0, "fps_avg": 0.0}},
           "samples": 10, "verdict": "FAIL"}
    problems = s.check_soak_summary(bad)
    assert any("running 占比" in p for p in problems)
    assert any("平均 fps" in p for p in problems)
    assert any("判定 FAIL" in p for p in problems)
    assert s.check_soak_summary({})  # 空汇总必须报错


def test_safe_base_rejects_ssrf():
    """SSRF 防护：仅允许本机 http 回环，拒外网/凭证/非法 scheme/路径。"""
    assert s.safe_base("http://127.0.0.1:8001") == "http://127.0.0.1:8001"
    assert s.safe_base("http://localhost:8000") == "http://localhost:8000"
    for bad in ("https://evil.example", "http://192.168.1.5/hook",
                "http://user:pw@127.0.0.1", "http://127.0.0.1/?x=1",
                "file:///etc/passwd", "http://127.0.0.1:99999"):
        try:
            s.safe_base(bad)
        except ValueError:
            continue
        raise AssertionError(f"未拒绝: {bad}")
    assert s.control_url("http://127.0.0.1:8001", "/api/cameras") == \
        "http://127.0.0.1:8001/api/cameras"
    try:
        s.control_url("http://127.0.0.1", "/etc/passwd")
    except ValueError:
        pass
    else:
        raise AssertionError("非 /api 路径未拒绝")


def test_soak_summary_warn_verdict_not_fatal():
    # WARN（重连/帧龄）不判失败，由人工复核
    d = {"cameras": {"1": {"running_rate_pct": 100.0, "fps_avg": 5.0}},
         "samples": 5, "verdict": "WARN"}
    assert s.check_soak_summary(d) == []


def test_mixed_check_via_stub(monkeypatch):
    """混合模式：python+rust 各一路 running → 通过；缺 rust → 报错。"""
    calls = []

    def fake_get(base, path, timeout=5):
        calls.append(path)
        if path == "/api/cameras":
            return [{"id": 1, "analyzer": "python"}, {"id": 2, "analyzer": "rust"}]
        return {"status": "running"}

    monkeypatch.setattr(s, "get_json", fake_get)
    assert s.check_mixed("http://127.0.0.1:8001") == []
    # 只有 python 路 → 缺 rust
    monkeypatch.setattr(s, "get_json", lambda b, p, timeout=5:
                         [{"id": 1, "analyzer": "python"}] if p == "/api/cameras"
                         else {"status": "running"})
    assert any("缺 running 的 rust" in p for p in s.check_mixed("http://127.0.0.1:8001"))


def test_event_rate_metrics_parse(monkeypatch):
    """metrics 有 analyzer 标签 → 通过；无标签 → 报观测缺失。"""
    class _Resp:
        def __enter__(self):
            return self

        def __exit__(self, *a):
            return False

        def read(self):
            return b'labsentinel_cameras{analyzer="python"} 1\n' \
                   b'labsentinel_cameras{analyzer="rust"} 1\n'

    monkeypatch.setattr(s.urllib.request, "urlopen", lambda *a, **k: _Resp())
    assert s.check_event_rate("http://127.0.0.1:8001") == []
    monkeypatch.setattr(s.urllib.request, "urlopen", lambda *a, **k: _Resp())
    # 无标签
    class _Empty(_Resp):
        def read(self):
            return b"# no metrics\n"

    monkeypatch.setattr(s.urllib.request, "urlopen", lambda *a, **k: _Empty())
    assert any("analyzer 标签" in p for p in s.check_event_rate("http://127.0.0.1:8001"))


def test_summary_path_resolution(tmp_path):
    """相对 summary 路径相对仓库根解析（不依赖 cwd）。"""
    p = Path("Resources/logs/soak-mixed.json")
    assert not p.is_absolute()
    resolved = s.ROOT / p
    assert resolved.parent.name == "logs"
