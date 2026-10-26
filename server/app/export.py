"""报表导出（任务15）：CSV（UTF-8 BOM，Excel 中文不乱码）+ PDF 月报（reportlab，中文字体可配）。

- 导出接口只读；事件量大时 CSV 用生成器流式返回。
- PDF 不含截图/人脸（合规），一页式月报：统计数字 + 每日柱状 + 合规脚注。
"""
import csv
import io
from datetime import date, datetime, timedelta

from . import stats as stats_mod
from .constants import TYPE_TEXT
from .config import settings

# 中文列名固定常量表（Excel 用户视角；禁止散落字符串）
CSV_EVENT_HEADERS = ["时间", "摄像头", "类型", "级别", "track", "详情", "状态", "处理人", "处理时间"]
CSV_STATS_HEADERS = ["日期", "合计", "L1", "L2", "L3"]
STATUS_TEXT = {"new": "未处置", "acked": "已确认", "false_positive": "误报", "ignored": "忽略"}


def _days_param(days: int) -> int:
    return max(1, min(int(days), 366))


def iter_events_csv(store, days: int, camera_id: int | None, status: str | None):
    """生成器：逐批取事件写 CSV 行（UTF-8 BOM 由调用方写入响应头）。"""
    days = _days_param(days)
    cutoff = (datetime.now() - timedelta(days=days)).isoformat(timespec="seconds")
    batch = 200
    last_id = None
    yield CSV_EVENT_HEADERS
    while True:
        rows = store.events_older_than_first(cutoff, camera_id=camera_id, status=status,
                                             before_id=last_id, limit=batch)
        if not rows:
            break
        for r in rows:
            last_id = r["id"]
            yield [
                r["ts"], f"#{r['camera_id']}", TYPE_TEXT.get(r["type"], r["type"]),
                f"L{r['level']}", r["track_id"], r["detail"],
                STATUS_TEXT.get(r.get("status") or "new", r.get("status") or "new"),
                r.get("handled_by") or "", r.get("handled_ts") or "",
            ]
        if len(rows) < batch:
            break


def stats_csv(store, days: int, camera_id: int | None = None) -> list[list]:
    days = _days_param(days)
    out = [CSV_STATS_HEADERS]
    for r in stats_mod.daily(store, days=days, camera_id=camera_id)["days"]:
        out.append([r["date"], r["total"], r["l1"], r["l2"], r["l3"]])
    return out


def _register_cn_font():
    """注册中文字体（默认微软雅黑；LS_PDF_FONT 可配）。失败时回退 Helvetica（中文会缺字）。"""
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.ttfonts import TTFont
    font_path = settings.pdf_font or r"C:\Windows\Fonts\msyh.ttc"
    name = "CN"
    try:
        pdfmetrics.registerFont(TTFont(name, font_path, subfontIndex=0))
    except Exception as e:  # noqa: BLE001
        print(f"[export] 中文字体注册失败（{font_path}）: {e}，PDF 中文将缺字")
        name = "Helvetica"
    return name


def monthly_pdf(store, month: str) -> bytes:
    """生成月报 PDF（yyyy-mm）。一页：标题/统计/每日柱状/合规脚注。"""
    from reportlab.graphics.shapes import Drawing, Rect, String
    from reportlab.lib.colors import HexColor
    from reportlab.pdfgen import canvas as pdfcanvas

    y, m = (int(x) for x in month.split("-"))
    start = date(y, m, 1)
    end = date(y + (m == 12), m % 12 + 1, 1)
    cutoff, until = start.isoformat(), end.isoformat()
    rows = store.events_between(cutoff, until)
    total = len(rows)
    by_level = {1: 0, 2: 0, 3: 0}
    unhandled = 0
    by_day: dict[int, int] = {}
    for r in rows:
        by_level[r["level"]] = by_level.get(r["level"], 0) + 1
        if (r.get("status") or "new") == "new":
            unhandled += 1
        by_day[int(r["ts"][8:10])] = by_day.get(int(r["ts"][8:10]), 0) + 1

    font = _register_cn_font()
    buf = io.BytesIO()
    c = pdfcanvas.Canvas(buf, pagesize=(595, 842))  # A4
    W, H = 595, 842
    c.setFont(font, 18)
    c.drawString(50, H - 60, f"LabSentinel 实验室安全月报（{month}）")
    c.setFont(font, 11)
    c.drawString(50, H - 85, f"生成时间：{datetime.now().strftime('%Y-%m-%d %H:%M')}")

    # 统计卡片
    c.setFont(font, 12)
    cards = [("事件总数", str(total)), ("L1 提示", str(by_level.get(1, 0))),
             ("L2 警告", str(by_level.get(2, 0))), ("L3 严重", str(by_level.get(3, 0))),
             ("未处置", str(unhandled))]
    x = 50
    for label, val in cards:
        c.setFillColor(HexColor("#1a2129" if val == "0" else "#4da3ff"))
        c.rect(x, H - 150, 88, 46, fill=1, stroke=0)
        c.setFillColor(HexColor("#ffffff"))
        c.setFont(font, 16)
        c.drawString(x + 10, H - 122, val)
        c.setFont(font, 9)
        c.drawString(x + 10, H - 140, label)
        x += 98

    # 每日柱状（SVG 思路直接画 Rect）
    days_in_month = (end - start).days
    maxv = max(by_day.values()) if by_day else 0
    c.setFillColor(HexColor("#000000"))
    c.setFont(font, 12)
    c.drawString(50, H - 190, "每日事件分布")
    base_y, chart_h, chart_w = H - 420, 200, W - 100
    c.setStrokeColor(HexColor("#888888"))
    c.line(50, base_y, W - 50, base_y)
    bw = chart_w / days_in_month
    for d in range(1, days_in_month + 1):
        v = by_day.get(d, 0)
        if v:
            bh = v / maxv * chart_h if maxv else 0
            c.setFillColor(HexColor("#4da3ff"))
            c.rect(50 + (d - 1) * bw + bw * 0.15, base_y, bw * 0.7, bh, fill=1, stroke=0)
        if d % 5 == 0 or d == 1:
            c.setFillColor(HexColor("#444444"))
            c.setFont(font, 7)
            c.drawString(50 + (d - 1) * bw, base_y - 12, str(d))

    c.setFont(font, 9)
    c.setFillColor(HexColor("#444444"))
    c.drawString(50, 60, "合规声明：本报告不含人脸照片与截图素材；仅含事件统计信息（80-risk-register C3）。")
    c.drawString(50, 46, "数据保留策略见 server/.env.example；人脸模块（如启用）默认关闭且需单独同意。")
    c.showPage()
    c.save()
    return buf.getvalue()
