# 移动端断点截图基线（375×812，DPR 2）

> 任务 34 产出 · playwright 实拍主线预览实例（:5173，浅色主题）。
> 用途：发布素材（移动端能力展示）+ 后续 UI 改动的**回归基线**。

## 截图清单与检查点

| 文件 | 页面 | 检查点 |
|---|---|---|
| `dashboard_375.png` | 仪表盘 | 统计卡片换行不溢出；事件流面板可读 |
| `events-drawer_375.png` | 事件查询 | 详情抽屉全屏（100vw）；表格在 overflow-x 容器内（**发现**：列被压缩成竖排文字，见下） |
| `zones_375.png` | 安全区配置 | 画布/快照不溢出 |
| `reports_375.png` | 报表 | 图表与导出按钮排布正常 |
| `cameras-alerts_375.png` | 摄像头（含告警设置区块） | 令牌/告警面板/表格可用 |

## 回归检查方法

改版或新增页面后，跑下面的脚本（playwright，gpu-venv 已装）核对两点：① 每页 `scrollWidth - clientWidth <= 0`（无横向溢出）；② 截图与基线人工比对。

```python
# docs/assets/mobile/回归检查要点（视口 375×812，device_scale_factor=2）
from playwright.sync_api import sync_playwright

with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_context(viewport={"width": 375, "height": 812},
                               device_scale_factor=2).new_page()
    page.goto("http://localhost:5173", wait_until="networkidle")
    for tab in ("仪表盘", "事件查询", "安全区配置", "报表", "摄像头"):
        page.click(f"nav >> text={tab}")
        page.wait_for_timeout(700)
        overflow = page.evaluate(
            "() => document.documentElement.scrollWidth - document.documentElement.clientWidth")
        print(tab, "溢出", overflow, "px")          # 期望 <= 0
        page.screenshot(path=f"check_{tab}.png", full_page=True)
```

## 已知发现（任务 34 记录，未修——17 已做修复轮）

- **事件表格列压缩**：375px 下表格列宽被压到极限（中文一字一行，竖排观感）。无横向溢出
  （overflow-x 容器生效）但可读性差。建议（如单开修复任务）：给 `.panel table` 设
  `min-width: 640px`，配合容器横向滚动即可恢复可读。
- 详情抽屉在移动端为 100vw 全宽（任务 17 scoped 媒体查询），截图确认工作正常。

## 截取方式

见 `docs/50-tasks/34-mobile-screenshots.md` 执行报告；关键技巧：抽屉打开态先点首行、
隐藏 naturalWidth=0 的破图 img、抽屉关闭后再点后续导航（遮罩会拦截点击）。
