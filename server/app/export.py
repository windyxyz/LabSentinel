"""报表导出（任务15）：CSV（UTF-8 BOM，Excel 中文不乱码）+ PDF 月报（reportlab，中文字体可配）。

- 导出接口只读；事件量大时 CSV 用生成器流式返回。
- PDF 不含截图/人脸（合规），一页式月报：统计数字 + 每日柱状 + 合规脚注。
"""
import csv
import io
import os
import zipfile
from datetime import date, datetime, timedelta
from pathlib import Path

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


# ---- 任务33：事件证据包 ZIP ----

EVIDENCE_MAX_EVENTS = 500
EVIDENCE_MAX_TOTAL_MB = 200


def _within_data_dir(path: str, data_dir) -> bool:
    """路径安全校验：必须在 data_dir 内（防穿越）。"""
    try:
        return os.path.commonpath([os.path.abspath(path), os.path.abspath(str(data_dir))]) \
            == os.path.abspath(str(data_dir))
    except ValueError:
        return False


def event_csv_row(r: dict) -> list:
    """单事件 → CSV 行（与任务15 列定义一致）。"""
    return [
        r["ts"], f"#{r['camera_id']}", TYPE_TEXT.get(r["type"], r["type"]),
        f"L{r['level']}", r["track_id"], r["detail"],
        STATUS_TEXT.get(r.get("status") or "new", r.get("status") or "new"),
        r.get("handled_by") or "", r.get("handled_ts") or "",
    ]


def build_evidence_zip(store, data_dir, camera_id=None, level=None, status=None,
                       start_ts=None, end_ts=None):
    """构建证据包 ZIP 字节流。

    超限抛 ValueError（调用方转 400）：事件 >500 条、文件总量 >200MB。
    只打包 data_dir 内文件（路径校验防穿越）；不含人脸原图（系统本就未存）。
    """
    events = store.events(limit=EVIDENCE_MAX_EVENTS + 1, camera_id=camera_id,
                          level=level, status=status, start_ts=start_ts, end_ts=end_ts)
    if len(events) > EVIDENCE_MAX_EVENTS:
        raise ValueError(f"筛选范围内事件 {len(events)} 条，超过打包上限 "
                         f"{EVIDENCE_MAX_EVENTS}——请缩小时间范围或增加过滤条件")

    files = []  # (arcname, abs_path)
    total_bytes = 0
    for e in events:
        for col, sub in (("snapshot", "snapshots"), ("video", "videos")):
            p = e.get(col)
            if p and os.path.exists(p) and _within_data_dir(p, data_dir):
                sz = os.path.getsize(p)
                total_bytes += sz
                files.append((f"{sub}/ev_{e['id']}{Path(p).suffix}", p))
    if total_bytes > EVIDENCE_MAX_TOTAL_MB * 1e6:
        raise ValueError(f"证据文件总量 {total_bytes/1e6:.0f}MB 超过 {EVIDENCE_MAX_TOTAL_MB}MB "
                         f"上限——请缩小时间范围或分批导出")

    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        # events.csv（UTF-8 BOM，列同任务15）
        csv_buf = io.StringIO()
        csv_buf.write("\ufeff")
        w = csv.writer(csv_buf)
        w.writerow(CSV_EVENT_HEADERS)
        for e in events:
            w.writerow(event_csv_row(e))
        z.writestr("events.csv", csv_buf.getvalue())

        # README.txt（生成信息 + 合规提示）
        n_snap = sum(1 for a, _ in files if a.startswith("snapshots/"))
        n_vid = sum(1 for a, _ in files if a.startswith("videos/"))
        z.writestr("README.txt", (
            "LabSentinel 事件证据包\n"
            f"生成时间: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}\n"
            f"筛选条件: camera_id={camera_id or '全部'} level={level or '全部'} "
            f"status={status or '全部'} start={start_ts or '不限'} end={end_ts or '不限'}\n"
            f"事件数: {len(events)}（截图 {n_snap} 个 / 视频 {n_vid} 个）\n"
            "合规提示: 证据包含个人信息与现场画面，请谨慎分发，"
            "按机构 retention 政策处理后删除。\n"))

        for arc, path in files:
            z.write(path, arc)

    return buf.getvalue(), {"events": len(events), "files": len(files),
                            "total_mb": round(total_bytes / 1e6, 2)}
