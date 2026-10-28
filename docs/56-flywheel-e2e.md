# docs/56 · 数据飞轮闭环演练报告（任务 41）

> 日期：2026-10-28 08:05。执行：flash（任务 41）。
> 目的：把"事件 → 标记误报 → 截图进标注池 → 抽帧复用"整条飞轮做一次端到端实跑留证。

## 1. 结论

**通过**：隔离实例（8001 + 临时库/目录）上全链路验证——事件产生、误报标记、
标注池副本**字节级一致**（复制非移动）、原截图完好、状态筛选可查、抽帧工具产出正常。
演练期间主库 `labsentinel.db` 零写入。

## 2. 时间线与证据

| 步骤 | 结果 |
|---|---|
| 隔离实例 | 8001 + `flywheel_test.db` / `flywheel_snapshots / videos / pool`；`LS_ADMIN_TOKEN` 未设置 ✓ |
| 造事件 | 文件摄像头（lab-walk-24076.mp4，autostart=false）→ 安全区 → enter(L2) → 启动 → **6s 出事件** |
| 事件 | `id=3 type=enter ts=2026-10-28T07:51:37` |
| 标记误报 | `POST /api/events/3/status {"status":"false_positive","handled_by":"flywheel-e2e"}` → `{"ok": true, "status": "false_positive"}` ✓ |
| 池副本 | `flywheel_pool/ev3_20261028_075137_cam1_enter.jpg`（命名符合 `ev{id}_{ts}_cam{cam}_{type}.jpg`） |
| 字节一致 | 池副本 = 235,113 bytes == API 原截图 235,113 bytes（**复制非移动**）✓ |
| 原截图 | `GET /api/events/3/snapshot` 仍 200 ✓ |
| 状态筛选 | `GET /api/events?status=false_positive` 命中该事件 ✓ |

## 3. 抽帧验证 + 一个真实 bug 修复

- `tools/frame_sampler.py --video ... --interval 1.0 --max 8` → **0 张**（与参数不符）。
- 排查：sampler 自报"保留 8 帧"但磁盘 0 张——**m0-venv 的 `cv2.imwrite` 对任意路径（含纯
  ASCII）静默返回 False**：cv2 的 DLL 依赖导入时的 PATH 注入（server 进程有 `app/__init__`
  注入所以正常；独立工具进程全败，效果演示 make_demo 的抽样图此前也从未写出过，同根因）。
- **修复**：`tools/frame_sampler.py` 改 `cv2.imencode(".jpg", frame)` + `buf.tofile(path)` →
  重跑 **8 帧jpg + manifest.csv 全部落盘** ✓。
- `tools/preannotate.py --help` 正常（含 --model）；未实跑预标注（避免与演练推理叠加，按任务说明）。

## 4. 发现与建议

- **R1（已顺手修复）**：`tools/frame_sampler.py` 的 `cv2.imwrite` 静默失败——独立工具进程
  统一用 imencode+tofile（本任务修复；`tools/preannotate.py` 若有 imwrite 建议同样处理，留建议）。
- **R2（建议任务）**：`tools/frame_sampler.py` 抽样图写完后可考虑同时输出缩略图拼版（便于人工
  快速筛帧），非缺陷。

## 5. 清理确认

flywheel_test.db / flywheel_snapshots / flywheel_videos / flywheel_pool / logs/flywheel_frames
全部删除；8001 已停；主库零写入 ✓。
