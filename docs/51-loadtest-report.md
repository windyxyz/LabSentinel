# 多路并发压测报告（任务 07）

> 状态：**进行中（修复前数据 + 根因诊断已落盘，修复后补测待整机稳定性确认）**
> 脚本：`tools/loadtest.py`（用法见文末）；原始采样：`docs/51-loadtest-raw.json`

## 1. 环境与方法

- 机器：AMD Ryzen AI 9 365（12 核）+ Radeon 880M 核显 + 31GB；后端 **DirectML**（日志确认 `backend=directml`）
- 服务端：压测专用实例（端口 8002，独立临时库/截图/视频目录）
- 信源：同一演示视频多路登记（`m0-spike/data/lab-walk-24076.mp4`，25fps/7.8s/196 帧，`LS_FILE_LOOP=true` 循环）
- 每路：安全区（大面积多边形）+ enter L2 / exit L1 规则（冷却 10s）；预热 30s 后每 30s 采样
- 指标：`/api/system` 各路 fps/status、服务端进程 CPU%（psutil 按端口反查 PID）、系统 CPU%、事件增量
- 注意：采样期间用户自己的服务端/桌面负载在跑（环境干扰如实保留）

## 2. 修复前实测（2026-10-26 00:16-00:27）

| 路数 | 分析FPS档 | 各路平均FPS | 最低FPS | 异常采样数 | 服务端CPU%均值/峰值 | 事件增量 | 事件/分钟 | 判定 |
|---|---|---|---|---|---|---|---|---|
| 1 | 5 | 20.6 | 20.3 | 0 | 105 / 109 | 146 | 29.2 | 达标 |
| 2 | 5 | 20.3 / 20.3 | 20.1 | 20 | 10 / 100 | 9 | 1.8 | **不达标（worker 全灭）** |

2 路组的原始采样显示：首个采样（00:22:23）两路 `running`、进程 CPU 204%；第二个采样起两路均为
`stopped`、CPU 掉落至 0，事件停在 202 条——**两个分析 worker 线程同时死亡**（服务端进程本身存活）。

## 3. 根因（已定位并修复）

**DirectML 的 InferenceSession 不支持多线程并发 Run。** 服务端所有摄像头共享全局单例引擎，
≥2 路分析 worker 同时 `sess.run` 即导致 DML 设备崩溃 → worker 线程死亡（本次压测）；
在更强并发压力下会进一步挂死 GPU 驱动 → 整机硬挂起（同日 3 次 Kernel-Power 41，bugcheck=0x0，
无蓝屏/转储/WHEA/TDR；00:30:47 另有 ACPI 事件「嵌入式控制器(EC)未响应」，提示固件层也参与其中）。

修复（`server/app/engine.py`，2026-10-26）：
1. 模块级 `_ACCEL_LOCK`：directml/vitisai 后端的所有推理（人检 + PPE，跨实例）**全局串行**；
   CPU 后端不加锁（ORT 本身线程安全）。单测 `test_core.py::test_accel_run_serializes` 用 FakeSess 验证。
2. `pipeline.py` worker 自愈：单帧推理异常只丢帧并打日志，不再杀死 worker 线程。

> 整机稳定性另见事件日志：三次硬挂起中后两次发生时压测负载并未运行，且机器装有两个虚拟显示适配器
> （Parsec / GameViewer，每次开机 WUDFRd 加载失败 0xC0000365）。用户历史记录中 v2 训练也曾因
> “CPU 争用死机”暂停——本机在**高负载组合下（CPU 满核 + GPU 推理 + 剪映/虚拟显示器）本身不稳**。
> 硬件侧建议：更新/回退 BIOS 与芯片组、核显驱动；停用不用的虚拟显示适配器；如仍不稳定先用
> `LS_BACKEND_ORDER=cpu` 排除 DML。

## 4. 修复后补测计划（待用户确认机器稳定后执行，降强度）

| 组 | 路数 | 分析FPS | 时长 | 说明 |
|---|---|---|---|---|
| A | 2 | 5.0 | 3 min | 验证串行锁后 worker 不再死亡（关键回归） |
| B | 4 | 2.5 | 2-3 min | 容量下限验证（M3 目标 ≥4 路的轻载档） |
| C | 4 | 5.0 | 3 min | 满负荷档（可选，视机器状态） |
| — | 8 | — | — | **跳过**：用户明确要求温和测试，8 路暂不压（容量规划按 A-C 线性外推并注明） |

- 每组之间冷却 ≥60s；服务端单实例；不与用户桌面重负载并发
- 补测完成后：更新第 2 节表格（加「修复后」分组列）、回填 roadmap M3 结论、清理临时库

## 5. 脚本用法

```bash
# 启动压测专用服务端（独立端口与临时数据）
cd server && LS_DB_PATH=D:/tmp/loadtest/db.db LS_SNAPSHOT_DIR=D:/tmp/loadtest/snaps \
  LS_VIDEO_DIR=D:/tmp/loadtest/vids LS_ANALYSIS_FPS=5.0 LS_FILE_LOOP=true \
  D:/LabSentinel/m0-venv/Scripts/python.exe -m uvicorn app.main:app --port 8002

# 压测（--fps 仅标注档位；换档需重启服务端）
python tools/loadtest.py --counts 2 --minutes 3 --fps 5.0 --port 8002 \
  --report docs/51-loadtest-report.md --json D:/tmp/loadtest/loadtest-raw.json
```
