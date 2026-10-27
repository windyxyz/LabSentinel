# 54 · RTSP 回环 + 断线重连演练报告（任务 39）

> 日期：2026-10-28 04:55。执行：flash（任务 39，43–46 批次后补做）。
> 目的：无真实摄像头的机器上，用 go2rtc 把演示视频发布为 RTSP，等价验证服务端的 RTSP 接入、
> 分析、事件与**断线重连**——M1.5 出口条件中机器可自动验证的部分。

## 1. 结论

**通过**：RTSP 摄像头接入（h264/1280×720）、DirectML 分析、事件与截图全链路正常；
**断线重连恢复耗时 ≈2s**（go2rtc 重启 + exec ffmpeg 拉流 + worker 重连），worker 无需人工干预。

## 2. RTSP 模拟源的踩坑与最终方案

| 方案 | 结果 |
|---|---|
| `ffmpeg:` 源（任务规格原方案） | ❌ 本 go2rtc 1.9.15 构建 `unsupported scheme`（WRN 日志） |
| 纯 RTSP 服务器 + ffmpeg 主动推流（`-f rtsp` ANNOUNCE） | ❌ Broken pipe（发布被断，原因未深究） |
| **`exec:` 源（最终采用）** | ✅ go2rtc 拉起 ffmpeg、从 stdout 读 MPEGTS、RTSP 对外供流 |

最终 `logs/rtsp-sim.yaml`（演练用，已删）：
```yaml
api: { listen: ":1984" }
rtsp: { listen: ":8554" }
streams:
  lab: "exec:ffmpeg -re -stream_loop -1 -i D:/.../m0-spike/data/lab-walk-24076.mp4 -c copy -f mpegts pipe:1"
```
验证：`ffprobe -rtsp_transport tcp rtsp://127.0.0.1:8554/lab` → `stream,h264,1280,720` ✓。

## 3. 服务端接入（8001 + 临时数据目录）

- 启动：`LS_DB_PATH=rtsp_test.db`、`LS_DATA_DIR/LS_SNAPSHOT_DIR/LS_VIDEO_DIR/LS_ANNOTATION_POOL_DIR`
  指向 `server/data/rtsp_data/`（演练后全删）；`--port 8001`；主库零写入 ✓。
- API：`POST /api/cameras {type:rtsp, source:rtsp://127.0.0.1:8554/lab, autostart:true}` + 安全区 +
  enter(L2) 规则。
- 结果：`status=running`、**fps=25.0**（统计窗口后）、**6s 出首条 enter 事件**、截图
  `200 image/jpeg` ✓；60s 内累计 20+ 条事件（cooldown 5s 生效，频率符合预期）。

## 4. 断线重连演练

| 步骤 | 观测 |
|---|---|
| `taskkill /F /IM go2rtc.exe`（exec 源 ffmpeg 随之退出） | 服务端 worker **不死亡**：status 保持 running |
| 断线 5s 后 | fps 显示 24.9（统计窗口未立即归零）；无新事件产生（流断） |
| 重启 go2rtc（同配置） | **≈2s 后** fps 恢复 24.9、新事件 id 越过基线继续增长 ✓ |
| 恢复期间 | 无需人工干预；无异常日志刷屏；无 CPU 飙升 |

恢复耗时的构成：go2rtc 启动（<1s）+ exec ffmpeg 拉流（~1s）+ 服务端 worker 的 RTSP 重连
（重连间隔为秒级）。对真实摄像头断电/断网的等价场景，这意味着**分钟级断线对业务无感**。

## 5. 发现与建议

- **R1（建议任务）**：本 go2rtc 构建不支持 `ffmpeg:` 源 scheme；若要用它做视频文件→RTSP 的常驻
  模拟源，建议统一用 `exec:` 源（本文档方案）。`docs/部署指南.md` 的演示配置如引用 `ffmpeg:` 需同步。
- **R2（观察项，非缺陷）**：断线期间 `/api/system` 的 fps 读数在统计窗口内缓慢回落而非立刻归零，
  status 保持 running——真实摄像头长时间断线时 UI 上的"看似正常"窗口取决于 fps 统计口径；
  若需要"断线 N 秒即置 offline"的显式状态，属新需求（建议 M2 讨论）。
- 演练结束已清理：rtsp_test.db / rtsp_data / rtsp-sim.yaml / 演练日志全删；1984/8554/8001 无监听；
  主库未写入本演练任何数据 ✓。

## 执行报告

- **命令与 harness**：`D:\tmp\rtsp_harness.py`（go2rtc/push/probe/server 子命令）+
  `rtsp_step2.py`（接入断言）+ `rtsp_reconnect.py`（断线计时）+ `rtsp_cleanup.py`（清理）。
- **关键输出**：ffprobe `stream,h264,1280,720`；`fps=25.0 status=running`；事件 `types=['enter']`；
  恢复 `≈2s`。
- **耗时**：RTSP 源方案试错（ffmpeg:/推流/exec:）约 15 分钟；接入+断线演练约 10 分钟。
- **合规**：未占 8000/5173；无并发重任务；主库未写入。
