# 任务 57 · 多路模拟 RTSP 压测（go2rtc 3 路 + DML 串行锁）

> 背景：真实多路摄像头暂不可用。用 go2rtc 把演示视频发布成 **3 路不同 RTSP 流**，在 8001 临时实例上验证多路并发：
> 串行推理锁是否生效、每路 fps/事件、CPU/GPU/内存是否稳定。**这是模拟压测，不替代真实摄像头 24h 验收。**

## 目标

1. **3 路 RTSP 模拟源**（go2rtc，避开 8000/5173）：
   - `logs/multi-rtsp.yaml`：`api :1984`、`rtsp :8554`，3 个 `exec:` 源（不同起点/文件）：
     - `lab1`: lab-walk 从头
     - `lab2`: lab-equip 从头
     - `lab3`: lab-walk（`-ss 2` 或不同文件，避免三路完全同步）
   - `tools/go2rtc.exe --config logs/multi-rtsp.yaml`；`ffprobe` 逐个确认 h264/分辨率。
2. **隔离服务端**：8001 + 临时 DB/SNAPSHOT/VIDEO/POOL；起 uvicorn。
3. **3 路接入**：API 创建 3 个 `type=rtsp` 摄像头 → 各 1 个安全区 + `enter` 规则 → 全部启动。
4. **采集 10 分钟**（`--duration 600` 或手动）：
   - 每 30s 采一次 `/api/system`：3 路 status/fps、today_counts；
   - 记录 `/api/events` 数量增长、API 延迟、进程 CPU/内存（PowerShell `Get-Process`）；
   - 观察服务端日志有无 analyze error、DML 异常、worker 停止；
   - 可选：`tools/camera_soak.py --camera-id <id>` 只测一路。
5. **判定**：
   - 3 路均 status=running、fps>0（允许低于单路，记录实际值）；
   - 事件持续产生、截图可访问；
   - 无 worker 死亡/整机卡死/驱动异常；CPU/内存无持续失控增长；
   - 若出现不稳定：立即降为 2 路/1 路复测，记录阈值，**不要硬撑**。
6. **清理**：停摄像头、删测试摄像头/区域/规则、停 8001/go2rtc、删临时数据与 yaml；确认端口空闲、主库零写入。
7. **报告**：`docs/65-multi-rtsp-sim-report.md`——配置、10 分钟指标表、事件数、CPU/内存曲线摘要、问题与建议
   （例如 DML 串行锁下的合理路数/降帧策略）。

## 约束

- **重负载一次只跑一个**；不训练、不跑其它推理/压测；不与便携包/真实摄像头测试并发。
- 只占 1984/8554/8001；不碰 8000/5173；临时数据目录并清理；主库零写入。
- 若机器出现卡顿/风扇狂转/驱动警告，立即停止并记录（安全优先，不追求跑满 10 分钟）。
- 不改服务端代码/阈值；如需改，写建议任务。

## 验收标准

- 3 路 RTSP 源可读；3 路摄像头均 running、fps>0、有事件/截图；
- 10 分钟（或安全缩短）指标与日志证据齐全；无 worker 死亡/死机；
- 清理完成、主库未动；报告含"模拟≠真实 24h"声明。

## 执行报告

**flash 2026-10-30 完成（10 分钟全程跑满，各项判定通过）**

- **配置**：go2rtc `logs/multi-rtsp.yaml` 3 路 `exec:` 源（lab1=lab-walk 从头、lab2=lab-equip 从头、
  lab3=lab-walk `-ss 2` 错开；ffmpeg `-c copy -f mpegts pipe:1`，方案沿用 docs/54 §2）；ffprobe 三路均
  `h264,1280,720` ✓。隔离服务端 8001 + `D:\tmp\ls57` 临时目录；3 路 rtsp 摄像头 + 全画幅安全区 +
  enter L2 规则（cooldown 5s）全部启动。
- **harness**：`D:\tmp\ls57\{setup,monitor,cleanup}.py`（setup 曾因漏调 start_go2rtc/start_server 报
  "go2rtc 未就绪"，排查后修复；zones 422 为 polygon 需传数组非 JSON 串，均已修）。
- **实测（21 采样点/601s）**：
  - 三路**全程 running**，fps 稳定值 25.0/30.0/25.0＝各自源帧率，**串行锁未造成降帧**；
  - 事件 61→648（+587/10min，三路持续 enter L2）；截图 cam1 289KB / cam2 205KB `200 image/jpeg`；
  - API /events 延迟 p50=20ms max=30ms；服务日志 718 行**零错误**（Traceback/analyze error/worker
    stopped 计数 0）；无 worker 死亡、无死机、无驱动异常。
  - **进程资源**（真实 uvicorn pid 现采补齐，monitor 曾误采 venv stub——缺陷如实记录）：全程平均
    **2.90 核**、现采 3.03 核；WS 2278MB / 峰值 2876MB；go2rtc 0.06 核/27MB。**3 路并发 ≈1 核/路**。
- **未降级复测**：3 路全程稳定，未触发"降为 2 路/1 路"条件。
- **清理**：3 摄像头删除、8001+go2rtc(含 ffmpeg) 停止、go2rtc.yaml 字节级还原、multi-rtsp.yaml 与
  临时数据删除；1984/8554/8001 全空闲；**主库 mtime 未变（零写入）** ✓。
- **报告**：[docs/65-multi-rtsp-sim-report.md](../65-multi-rtsp-sim-report.md)（含 R1 harness 坑 /
  R2 多路上限留真机 / R3 H.265 转码开销提示；含"模拟≠真实 24h"声明）。
- **遗留**：无阻塞。4 路以上外推未实测，留给真实摄像头批次。
