# docs/assets/ · README 截图素材

> 由任务 20 的 playwright 截图脚本生成（1280×720，WinUI 浅色主题）。
> 更新方式：启动服务端+Web 后运行截图脚本，或手动截图覆盖同名文件。

| 文件 | 对应页面 | 内容要点 |
|---|---|---|
| `dashboard.png` | 仪表盘 | 统计卡片（后端/今日事件/未处置/级别/实时通道）+ WebSocket 事件流面板 |
| `events-drawer.png` | 事件查询 | 事件表格（级别色标）+ 右侧详情抽屉（处置按钮：确认/误报/忽略）——展示任务 12/17 成果 |
| `zones.png` | 安全区配置 | 多边形画布与规则配置入口 |
| `reports.png` | 报表 | 每日趋势折线（L1/L2/L3 分线）+ 导出按钮（CSV/PDF，任务 15） |
| `face-disabled.png` | 人脸管理 | 默认关闭状态的合规提示界面 |

## 截取方式

```bash
# 1. 启动服务端与 Web（scripts\start-all.bat 或手动）
# 2. 运行截图脚本（playwright，1280×720，仓库外任意位置）
#    逐页 click nav 标签 → page.screenshot(docs/assets/<name>.png)
```

## 注意

- 截图仅用公开演示素材，**不得包含真实人脸/真实实验室敏感画面**（70-release-plan 约束）
- 发布前如 UI 有改动（主题/新增页面），重新截图保持 README 素材与实际一致
- README.md 的截图墙引用本目录相对路径，改文件名需同步改 README


## 移动端断点截图（375×812，DPR 2）

`mobile/` 目录为移动端基线（任务 34）：dashboard / events-drawer / zones / reports / cameras-alerts
五张，含溢出检查结论（全部无横向溢出）与回归检查脚本要点——详见 [mobile/README.md](mobile/README.md)。
