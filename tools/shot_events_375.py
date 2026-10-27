# 任务37 回归：375px 事件页截图 + 溢出检查（需 vite preview 或 dev 已起）
# 用法: python tools/shot_events_375.py [base_url]
import sys
from playwright.sync_api import sync_playwright

base = (sys.argv[1] if len(sys.argv) > 1 else "http://localhost:5174").rstrip("/")
out = r"D:\User\Desktop\TOOLS\监控识别\docs\assets\mobile\events-table_375.png"

with sync_playwright() as p:
    b = p.chromium.launch()
    pg = b.new_page(viewport={"width": 375, "height": 812}, device_scale_factor=2)
    pg.goto(base + "/", wait_until="networkidle")
    # App.vue 无 vue-router：nav 是 <a>（非 button），点击"事件查询"标签（key=events）
    pg.locator("nav a", has_text="事件查询").click()
    pg.wait_for_timeout(1200)
    pg.wait_for_timeout(800)
    m = pg.evaluate("() => ({sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth})")
    print("page overflow:", m["sw"] - m["cw"], "px  (must be 0)")
    # 表格容器内部应可横向滚动
    tscroll = pg.evaluate("""() => {
        const t = document.querySelector('.events-table');
        const wrap = t ? t.closest('.panel') : null;
        return wrap ? {sw: wrap.scrollWidth, cw: wrap.clientWidth} : null;
    }""")
    print("table wrap scrollable:", tscroll)
    pg.screenshot(path=out, full_page=False)
    print("saved", out)
    b.close()
