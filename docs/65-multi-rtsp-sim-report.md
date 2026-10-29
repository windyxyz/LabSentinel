# 65 · 多路模拟 RTSP 压测报告（任务 57）

> 日期：2026-10-30。执行：flash。**模拟压测，不替代真实摄像头 24h 验收**（摄像头 24h 清单仍是发布硬阻塞）。
> 结论先行：**DirectML 串行锁下 3 路并发 10 分钟全程稳定**——三路 fps 全程等于源帧率、
> 事件持续产出、API 延迟 p50=20ms、日志零错误、无 worker 死亡；服务端平均 2.9 核 / 峰值内存 2.9GB。

## 1. 配置

- **3 路 RTSP 模拟源**（go2rtc 1.9.15，`logs/multi-rtsp.yaml`，`exec:` 源 + ffmpeg `-c copy` 重封装，
  方案沿用 [54 回环报告](54-rtsp-loopback-report.md) §2——本构建不支持 `ffmpeg:` scheme）：

| 流名 | 源文件 | 起点 | ffprobe 实测 |
|---|---|---|---|
| lab1 | lab-walk-24076.mp4 | 从头 | h264,1280,720 |
| lab2 | lab-equip-21453.mp4 | 从头 | h264,1280,720 |
| lab3 | lab-walk-24076.mp4 | `-ss 2` 错开 | h264,1280,720 |

- **隔离服务端**：uvicorn :8001（`LS_DB_PATH/LS_DATA_DIR/LS_SNAPSHOT_DIR/...` 全指向 `D:\tmp\ls57\data`，
  venv python 启动）；主库零写入（mtime 校验）。
- **接入**：3 个 `type=rtsp` 摄像头 + 各 1 全画幅安全区 + `enter` L2 规则（cooldown 5s）→ 全部启动。

## 2. 10 分钟指标（21 个采样点 / 30s 间隔，metrics.jsonl 备查）

| 指标 | 结果 |
|---|---|
| 三路 status | **全程 running**（21/21 采样无一例外） |
| fps 后半段（稳定值） | cam1 **25.0**（24.8–25.1）＝源帧率；cam2 **30.0**（29.8–30.2）＝源帧率；cam3 **25.0**（24.8–25.6）＝源帧率 → **串行锁未造成降帧** |
| 事件增长 | 61 → 648（10 分钟 **+587**，三路持续产出 enter L2；cooldown 5s 生效） |
| 截图 | cam1 `200 image/jpeg 289KB`、cam2 `200 image/jpeg 205KB` |
| API 延迟 /events | p50 **20ms**、max 30ms（压测负载下无劣化） |
| 服务日志 | 718 行，**Traceback/analyze error/worker stopped 计数 = 0** |
| 异常采样点 | 无（API 采样错误 0） |

## 3. 进程资源（真实进程）

> ⚠️ harness 缺陷如实记录：monitor 的进程统计采到了 venv `python.exe` **启动器 stub**（4MB/0 CPU），
> 真正的 uvicorn 是它的子进程。真实数据由压测结束后对真身 pid 的现采补齐
> （`GetProcessTimes` 总量/进程年龄 + 60s 现采，此时压测负载仍在运行）：

| 进程 | CPU | 内存 |
|---|---|---|
| uvicorn（3 路推理） | **全程平均 2.90 核**（cpu_total 2388s / age 822s）；收尾现采 60s = **3.03 核** | WS **2278MB**、峰值 **2876MB** |
| go2rtc（3 路转推） | p50 0.06 核 / max 0.09 核（`-c copy` 仅重封装，轻载） | 26–27MB 稳定 |

解读：**3 路并发 ≈ 1 核/路**（DirectML 走 880M 核显 + CPU 管线/编码），内存以模型+帧缓冲+DML arena 为主。
12 核机器上 3 路占用 ~25%，外推 4 路约 4 核——但该外推**未实测**，真实路数上限需真机 24h 验证。

## 4. 问题与建议

- **R1（harness 修复项）**：对 venv 启动的 python 进程做资源采样时必须取真实子进程
  （venv 的 python.exe 是转发器）。`tools/camera_soak.py` 如加进程监控需注意同坑。
- **R2（观察）**：三路 fps 与源帧率一致，说明 3 路场景串行锁无瓶颈；瓶颈特征未出现，
  多路吞吐的真正上限（含 NPU/DML 取舍）建议留到真实摄像头批次再测（与 docs/63 M2 备忘录 D 方向衔接）。
- **R3**：ffmpeg `exec:` 源用 `-c copy` 时 CPU 开销可忽略；若源是 H.265 需转码，CPU 会显著上升——
  真机子码流建议固定 H.264 主流配置。

## 5. 清理证据

- 删除 3 个测试摄像头（连带区域/规则级联）；停 8001/uvicorn、停 go2rtc（/T 连带 ffmpeg 子进程）；
- `tools/go2rtc.yaml` 已还原（压测前字节备份）；`logs/multi-rtsp.yaml` 已删；`D:\tmp\ls57\data` 已删
  （metrics/logs 留档 `D:\tmp\ls57` 备查）；
- 端口 1984/8554/8001 全部空闲；**主库 mtime 未变（零写入）** ✓。
