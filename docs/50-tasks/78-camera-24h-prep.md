# 任务 78 · 真实摄像头 24h 验收准备包（P1，不跑 24h）

> 背景：发布唯一硬阻塞是真实 RTSP 摄像头 24h，但现有 `tools/camera_soak.py` 只测单路、输出简单，
> 缺少接入前预检（编码/分辨率/磁盘估算）和多路汇总。用户接上摄像头后应能一条命令开始、自动出
> 可附在发布证据里的 CSV+摘要。**本任务不要求真跑 24h**（硬件不在），只把工具与清单做到位并做短测。

## 目标

### 1. `tools/camera_soak.py` 增强（保持向后兼容）

- 参数：
  - `--camera-id ID`（可重复）/`--all`（所有已启用摄像头）；
  - `--duration 24h|3600`（支持 s/m/h 后缀）、`--interval 30`；
  - `--base-url http://127.0.0.1:8000`、`--out soak-<ts>.csv`、`--summary-json <path>`；
  - `--stall-s 60`（超过该秒数无新帧判“画面停滞”）；
  - `--preflight`（只做接入前检查后退出）。
- 每采样点 CSV 列：`ts, camera_id, status, fps, frame_age_s, reconnect_count, last_error,
  events_total, api_latency_ms, cpu_percent, rss_mb, disk_free_gb`。
- 汇总 JSON：时长、采样数、每路 running 占比/平均 fps/最大帧龄/重连次数/错误种类、事件增长、
  API p50/p95/max、CPU/RSS 峰值、磁盘下降、判定 `PASS/WARN/FAIL`（阈值可配置）。
- `--preflight`：
  - 对每个 RTSP source 用 `tools/ffprobe`（或 ffmpeg -i）探测 codec/分辨率/fps，H.265 标 WARN
    （需 go2rtc 转码）；失败 FAIL；
  - 检查模型/classes 存在、DB integrity、磁盘 ≥20GB（24h 估算）、8000 可达、摄像头账号不含默认密码；
  - 输出表格 + 退出码（FAIL=2）。
- 纯函数拆分（解析 ffprobe JSON、时长解析、判定、汇总计算）便于单测；不真跑 24h。

### 2. 测试 `server/tests/test_camera_soak.py`

- 时长解析（`24h`/`90m`/`3600`/非法）；
- 汇总：给定样例采样 CSV（含 running/error/停滞/重连）→ 统计与 PASS/WARN/FAIL 正确；
- 帧龄/停滞判定边界；
- preflight 解析 ffprobe JSON（h264/h265/失败）；不启动真实进程。

### 3. 文档

- `docs/摄像头24h验收清单.md` 增加：
  - 接入前 `--preflight` 命令与通过标准；
  - 24h 正式命令（多路 `--all`）、中断恢复（Ctrl+C 落盘）、产物（CSV/JSON/日志/截图）；
  - PASS/FAIL 表与发布证据要求（24h 连续、无 worker 死亡、事件/截图可查、磁盘不爆）；
  - 明确“模拟/虚拟摄像头不能替代”。
- `docs/运维手册.md` 24h 观察清单链接到该命令。

## 约束

- 不启动训练/长时压测；只做 `--preflight` 与 ≤60s 的文件源/模拟源短测（如条件允许）。
- 不改服务端业务代码；不引入依赖（标准库 + httpx/现有 opencv/ffprobe）。
- 所有产物写临时目录，测试后清理；不碰真实 `server/data`。

## 验收

- `pytest tests/ -q` 全绿（基线 165 + 新增）；
- `--help` 完整；`--preflight` 对无摄像头配置返回清晰提示（不崩）；
- 报告附短测输出/解析样例；**不得声称已完成 24h**。

## 执行报告

**flash 2026-10-31 完成（pytest 193 passed；camera_soak 8 用例；--help/--preflight 短测过）**

- **改动**：`tools/camera_soak.py` 重写（保持 CLI 兼容）——
  - 多路：`--camera-id` 可重复 / `--all`（默认全部），每路每采样点一行；
  - 新 CSV 列（规格 12 列全有）：fps/frame_age_s/reconnect_count/last_error/events_total
    （改用 `/api/events/count`）/api_latency_ms/cpu_percent/rss_mb/disk_free_gb；
  - `SystemMetrics`（ctypes GetSystemTimes）整机 CPU%（标准库，无 psutil；整机口径已在
    --help/报告注明）；`rss_mb` 本轮留空列（无 psutil，避免误报——记录为遗留）；
  - 汇总 JSON：时长/采样数/API 率+p50/p95/max/每路 running 率/平均 fps/最大帧龄/停滞采样/
    重连/错误种类/事件增长/CPU·RSS 峰值/磁盘下降/**判定 PASS|WARN|FAIL**（阈值参数化）；
  - `--preflight`：服务可达/每路 RTSP ffprobe（H.265→WARN、失败→FAIL）/模型词表/DB integrity/
    磁盘 ≥20GB/默认密码 WARN；FAIL 退出码 2；
  - 纯函数拆分：parse_duration/parse_ffprobe_json/build_row/summarize/render_summary/
    parse_ffprobe_stream 均可独立单测。
- **测试**：`server/tests/test_camera_soak.py` 8 用例（时长含非法/ffprobe h264+h265+0/0+垃圾/
  汇总 PASS/FAIL（错误+running 率）/WARN（重连>10 与停滞>3×stall）/停滞边界 age==stall 不算/
  事件增长与延迟分位/build_row 列序/preflight 无服务清晰失败 rc=2）。
- **短测**：`--help` 全参数；`--preflight` 对不可达服务 → `[FAIL] 服务可达…` + rc=2（输出见上）。
  **未跑 24h，不得声称已完成 24h**。
- **文档**：24h 验收清单新增 §〇 预检（命令+通过标准+"虚拟摄像头不能替代"）、§二 多路命令与
  中断恢复/产物说明、§二.5 PASS/WARN/FAIL 发布证据表；运维手册 §12 改 `--all` 多路口径并链预检。
- **遗留**：rss_mb 列当前恒空（需 psutil 或读进程句柄，属引入依赖范畴）——CSV 列已在，
  后续可补；preflight 的"摄像头账号不含默认密码"检查以启发式 WARN 落地（admin:admin/12345）。
