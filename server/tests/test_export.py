"""单元测试：任务15 报表导出（CSV 列/BOM/边界 + PDF 可开与中文）。运行: pytest tests/ -q"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import pytest  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

from app import api as api_mod  # noqa: E402
from app.main import app  # noqa: E402
from app.store import Store  # noqa: E402


@pytest.fixture()
def client(tmp_path, monkeypatch):
    s = Store(path=tmp_path / "t.db")
    monkeypatch.setattr(api_mod, "store", s)
    e1 = s.add_event(1, "enter_no_ppe", 3, 7, "进入安全区「A」未穿实验服")
    e2 = s.add_event(1, "exit", 1, 7, "离开安全区「A」")
    s.set_event_status(e2, "false_positive", handled_by="op1")
    return TestClient(app)


def test_events_csv_headers_and_bom(client):
    r = client.get("/api/export/events.csv", params={"days": 30})
    assert r.status_code == 200
    raw = r.content
    assert raw.startswith(b"\xef\xbb\xbf")  # UTF-8 BOM（Excel 中文不乱码）
    text = raw.decode("utf-8-sig")
    lines = [ln for ln in text.splitlines() if ln]
    assert lines[0] == "时间,摄像头,类型,级别,track,详情,状态,处理人,处理时间"
    assert len(lines) == 3  # 表头 + 2 事件（新→旧）
    assert "离开安全区" in lines[1] and "误报" in lines[1] and "op1" in lines[1]
    assert "进入安全区未穿实验服" in lines[2] and "未处置" in lines[2]


def test_csv_days_clamp(client):
    assert client.get("/api/export/events.csv", params={"days": 99999}).status_code == 200
    r = client.get("/api/export/stats.csv", params={"days": -5})
    assert r.status_code == 200
    lines = [ln for ln in r.content.decode("utf-8-sig").splitlines() if ln]
    assert len(lines) == 2  # 钳到 1 天：表头 + 1 行


def test_stats_csv_shape(client):
    r = client.get("/api/export/stats.csv", params={"days": 7})
    lines = [ln for ln in r.content.decode("utf-8-sig").splitlines() if ln]
    assert lines[0] == "日期,合计,L1,L2,L3"
    assert len(lines) == 8  # 表头 + 7 天


def test_monthly_pdf_valid_and_cn(client):
    r = client.get("/api/export/monthly.pdf", params={"month": "2026-10"})
    assert r.status_code == 200
    assert r.content[:5] == b"%PDF-"
    # 中文字体嵌入检查：PDF 内应有 TrueType 字体描述（简单启发式）
    assert b"FontFile" in r.content or b"Identity-H" in r.content or b"CN" in r.content
    # 非法月份 → 400
    assert client.get("/api/export/monthly.pdf", params={"month": "2026/10"}).status_code == 400
