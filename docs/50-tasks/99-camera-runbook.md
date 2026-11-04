# 任务 99 · 摄像头接入 Runbook（P2，纯文档）

> 背景：部署指南有 RTSP 模板，但没有“从拆箱到 24h 验收”的一条龙操作手册；用户接真机时容易在
> 网络/编码/账号/区域/告警环节卡住。本任务写 `docs/摄像头接入Runbook.md`，面向现场实施人员。

## 内容要求

1. **前置条件**：摄像头与主机同网段/可路由；主机有线优先；电源/POE；已知 IP/账号（只读子码流账号）；
   主机磁盘 ≥ 20GB；时间同步（NTP）。
2. **接入前预检**：`tools/camera_soak.py --preflight`（78 完成后）或 ffprobe 手动命令；
   H.264 vs H.265（go2rtc 转码代价）、分辨率/帧率/码率；RTSP 连通性测试（ffprobe/ffplay）；
   账号权限最小化；确认子码流地址。
3. **添加摄像头**：页面步骤（类型 RTSP、source 模板、不自动启动先配区域）；USB 用设备下拉（60）；
   保存后状态 running/fps/画面。
4. **画安全区与规则**：全画幅 vs 特定区域；enter/enter_no_ppe/occupancy_over 语义；level/cooldown/
   时间段；多路复制（80 完成后）。
5. **告警验证**：/api/alerts/status 配置、一键自测、真实事件触发、失败重发、静默（81）/升级（82）。
6. **稳定性观察**：先 1 小时，再 24h；`camera_soak --all` 命令、产物 CSV/JSON、判定阈值、
   中断恢复；看 Dashboard 摄像头卡（frame_age/重连）。
7. **验收清单**：24h 连续、无 worker 死亡、事件/截图可查、磁盘不爆、日志无 ERROR 洪峰、
   CPU/内存无持续增长；PASS/FAIL 表（与 `摄像头24h验收清单.md` 对齐）。
8. **故障速查表**：连不上（网络/防火墙/账号/路径/编码）、能连不能出事件（区域/规则/时间/模型类别）、
   画面卡顿（码率/子码流/DML 负载）、误报多（阈值/区域/夜红外）、告警不发（渠道/静默/过滤）。
9. **回滚/停止**：停摄像头、恢复旧配置（79 导入）、卸载/保留数据。

## 约束

- 只写文档，不改代码；命令必须与仓库现状一致（写前核对 tools/scripts/config/端点）；
- 不编造 24h 结果；未完成的项目写“待执行”；
- 面向现场：每步有“预期结果/失败先查什么”；Windows 路径完整可复制；
- 链接到 59/64/65/78/81/82/运维手册/24h清单。

## 验收

- 文档存在且 README 文档索引 + 部署指南 + 运维手册均有入口；
- 相对链接检查可达；命令 `--help` 核对（camera_soak/ffprobe/backup_data）；
- 报告附章节结构与命令核对表。

## 执行报告

**flash 2026-11-04 完成（纯文档；release_check 相对链接 312 条全过）**

### 交付

- 新文档 `docs/摄像头接入Runbook.md`，九章：前置条件表 / 接入前预检（ffprobe 完整路径 +
  `camera_soak --preflight`）/ 添加摄像头（USB 下拉、先配区后启动）/ 区域与规则
  （三类型语义 + 复制接口）/ 告警验证（自测→真实→静默/升级）/ 稳定性观察
  （1h→24h 命令、产物、判定阈值、守护重试说明）/ 故障速查表（8 症状）/ 验收清单 / 回滚停用。
- 每步含"预期结果/失败先查什么"；Windows 路径完整可复制（`tools\ffmpeg\ffprobe …`）。

### 命令/链接核对表（写前逐一实测/核对）

| 项 | 现状 | 结论 |
|---|---|---|
| camera_soak 帮助文本 | `--preflight/--all/--duration(24h/90m/3600)/--interval/--out/--summary-json/--stall-s` 与实测 `--help` 一致 | ✓ |
| ffprobe | `tools/ffmpeg/ffprobe.exe` 存在 | ✓ |
| 备份脚本 | `scripts/backup_data.py` 存在（备份 API 口径） | ✓ |
| RTSP 模板 | 部署指南（海康 101/102、大华 subtype、宇视 video1）引用一致 | ✓ |
| 端点 | usb-devices/alerts status/alert-test/alert-silences/copy/config import/diagnostics summary/单路 status 均在 api 在位 | ✓ |
| 文档链接 | 部署指南/FAQ/运维手册/24h清单/35-api-reference/标注手册/59-portable-install 均存在；release_check 312 条全过 | ✓ |
| 未完成项标注 | 24h 结果明示"不含未实测结论"；验收表外链 24h 清单 | ✓ |

### 入口

- README 文档索引新增一行；部署指南 §3「摄像头接入」顶部指向；运维手册 §4 顶部指向。

### 遗留

- 无。注意：Runbook 引用的守护重试/处置质量卡来自本日任务 109/110/114/123/102 已落地实现。
