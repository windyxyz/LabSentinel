# 55 · 交接快照（2026-10-28 09:15，主线暂停点）

> 本文件是**上下文压缩的落点**：新会话/新协作者先读这一份，再按链接进细节。
> 当前状态：M1/M1.5 功能、回归、便携分发包基本就绪；**主线已暂停**，flash 子代理仍在执行 51–55/57。
> 发布唯一硬阻塞：**真实 RTSP 摄像头 24h 验证**（用户侧硬件）。

## 一、总体状态

| 维度 | 状态 |
|---|---|
| 服务端/Web | ⏸ 已停止；8000/8001/1984/8554/5173 均无监听 |
| 测试 | ✅ **pytest 91 passed**（含 USB 源解析、单端口 SPA、种子单源）；`npm run build` 通过（gzip 42.77KB） |
| 模型 | ✅ v2 已接入：`m0-spike/yolo11n_ppe.onnx`（11 类）+ classes.txt；v1 备份 `*_v1.*` |
| 便携包 | ✅ `dist/LabSentinel-portable-20261028.zip` **265MB**（08:48 重建，含 media + seed_demo）；安装后 **709MB**；首启自动播种 4 路演示摄像头（实测） |
| 启动 EXE | ⏳ 任务 55 待做（C# winexe；csc 已确认可用）；当前只有 `启动.bat`/`pythonw` |
| 部署 | ✅ 便携包（普通用户）+ `一键启动.bat`/`deploy.py`（开发模式，无 Python 时自动改启便携包） |
| Git | 最近提交 `ec5443a`（USB/OBS）；**工作区有 flash 未提交的 51/52 代码**（见 §五，勿回滚） |
| 协作 | 旧 5 分钟 cron 已删除（熔断后无法自删，主会话手动删）；当前按需批量派发 |
| 发布 | ⛔ 被真实摄像头 24h + 模型停止线阻塞；其余自动可验项全绿 |

## 二、任务状态（01–57）

- **01–39：已完成**（详见各任务文档执行报告）。
- **40/47/41/42/48/49/50：已交付并主线初验**（pytest 85/build/首启播种/soak/清理/导出/文档）。
- **51 播种单源化：已交付待提交**（flash，08:43）：新增 `scripts/seed_demo.py`，
  deploy/portable/build 共用；`server/tests/test_seed_demo.py`；pytest 90；便携包已重建。
- **52 CI 便携包流水线：进行中**（flash）：`.github/workflows/portable.yml` 已落盘（08:43），执行报告未写。
- **53 M2 方案备忘录：排队**（未开工）。
- **54 文档口径统一：排队**（未开工）。
- **55 启动 EXE：排队**（需与 51 串行，51 已完成，可开工）。
- **56 本机 USB/OBS 虚拟摄像头：主线已完成**（09:05，提交 `ec5443a`）：
  - 物理 HD Webcam index 0（当前黑屏/遮挡）→ status=running、fps 12.5、快照 200；
  - OBS Virtual Camera index 1（pyvirtualcam 播 lab-walk）→ **fps 29.6、3 条事件、截图 200/196KB**；
  - 修复 `usb` 源 `"0"/"1"` 被当文件名（`capture_source()`）与 FPS=-1 兜底；
  - 新增 `tools/list_cameras.py`、`tools/virtual_cam_feed.py`；报告 `docs/64-usb-virtual-camera-report.md`；
  - **仅设备链路，不替代真实 RTSP 24h**。
- **57 多路模拟 RTSP 压测：已派发**（`docs/50-tasks/57-multi-rtsp-sim.md`）：go2rtc 3 路 + 8001，
  10 分钟受控；模拟非真实 24h；排在 55 之后。

## 三、最近提交

```
ec5443a feat: USB/OBS 虚拟摄像头接入验证 + usb 源解析修复
764094a docs(dispatch): 新增任务55 启动EXE（无黑框，脚本/EXE双入口）
60072ee docs(dispatch): 复审后新批 51-54（播种单源/CI便携包/M2备忘录/文档口径）
db19dc9 fix+docs: 项目复审与三处兜底修复（preannotate/一键启动/便携移动自愈）
fe8cfbc feat: 任务40/47/41/42/48/49/50 交付
9810645 docs(dispatch): 批量任务补充并行约束
```

## 四、本轮关键修复（复审 + 摄像头）

1. `tools/preannotate.py`：预览图改 imencode+bytes，中文路径不再静默丢失。
2. `一键启动.bat`：无 Python 且存在便携包 → 自动改启便携版；有 Python 时提示优先便携包。
3. `portable_launcher.py`：新增 `repair_media_paths()`，便携包移动/改名后按 media 文件名自愈。
4. `server/app/pipeline.py`：`capture_source()` 修 USB 设备号；USB FPS=-1 兜底 30。
5. SPA 静态回退不再吞未知 `/api/*`（任务43，测试覆盖）。
6. README：Python 3.10+、便携/源码双路径播种说明。

## 五、未提交工作（重要，勿回滚）

`git status` 当前（flash 51/52 在途）：

```
 M docs/50-tasks/51-seed-single-source.md
 M scripts/build_portable.py
 M scripts/deploy.py
 M scripts/portable_launcher.py
?? .github/workflows/portable.yml
?? scripts/seed_demo.py
?? server/tests/test_seed_demo.py
?? docs/50-tasks/批次断点-51至55.md
 M docs/50-tasks/调度看板.md / 派发状态.json（主线刚更新，含 57）
```

- **不要**为了"干净"回滚这些文件；它们属于任务 51/52 交付，等 flash 写执行报告后，
  主线复跑 `pytest tests -q`（应 ≥91）→ 提交。
- 51 与 55 都改 `scripts/build_portable.py`，必须 **51→55 串行**；52/53/54 可并行。
- 53/54 纯文档；55 需要 `csc.exe`（已确认 `C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe`）。

## 六、发布差距（v0.1.0-alpha）

1. **真实 RTSP 摄像头 24h**（唯一硬阻塞）+ 模型停止线（Lab Coat ≥60%、误报 ≤20%/天）。
   - 工具：`tools/camera_soak.py` + `docs/摄像头24h验收清单.md`。
   - 虚拟摄像头/模拟 RTSP **不能**替代。
2. 干净 Windows VM：安装/启动/卸载/自启（46 在本机自启被环境阻塞）；无 Python/Node。
3. GitHub 远端 + Actions 实际跑 52（当前本地 git、无远端）；Release 附件/zip+sha256/公告定稿。
4. 代码签名（可选，影响 SmartScreen/杀软）。
5. 不在 alpha 范围：Android APK（仅 Flutter 骨架）、iOS/HarmonyOS、人脸实测（默认关）、
   真实多路吞吐、NPU 打包、GB28181；告警真实 webhook 未实测。
6. 未完善项：日志轮转、自动更新、备份恢复、多用户/角色、导出 500 上限/PDF 页级解析/磁盘紧急清理。

## 七、恢复工作建议（新会话直接照做）

1. 读本文件 + `docs/50-tasks/调度看板.md` + `docs/62-project-audit-2026-10-28.md`。
2. `git status`：若 51/52 已有执行报告，跑全量 pytest/build 后提交；再让 flash 继续 53/54/55/57。
3. 55 完成后：`python scripts/build_portable.py --zip`，用 `dist/LabSentinel-portable-*.zip` 做干净 PATH 首启；
   安装器快捷方式应指向 `LabSentinel.exe`。
4. 57 完成后看 `docs/65-multi-rtsp-sim-report.md`，确认 DML 串行锁下 3 路稳定。
5. 用户接真实摄像头后：按 `docs/摄像头24h验收清单.md` 跑 24h → 通过才发布 v0.1.0-alpha。
6. **不要**重新创建每 5 分钟自动巡检除非先修"心跳"（任务文档 mtime 会误判；应把 `server/app`、
   `web/src`、`scripts` 最新 mtime 也算进度，且 cron 无法自删，需主会话删）。

## 八、环境要点

- 项目根 `D:\User\Desktop\TOOLS\监控识别`；首选 venv 项目 `.venv`（py3.12，`scripts\venv_path.txt` 相对路径）；
  旧装兼容 `D:\LabSentinel\m0-venv`（py3.14，训练/工具）；NPU venv `D:\LabSentinel\npu-venv`。
- 普通用户：便携包 `安装.bat`/`启动.bat`（55 后加 `LabSentinel.exe`）；开发：`一键启动.bat`/`scripts/start-all.bat`。
- 单端口：`npm run build` 后只开 8000；`LS_WEB_DIST` 可覆盖。
- OBS 虚拟摄像头联调：`pip install pyvirtualcam`（已装 m0-venv）；`python tools/virtual_cam_feed.py`；
  服务端 `type=usb, source=1`；设备索引用 `tools/list_cameras.py` 枚举。
- 铁律：DML 禁并发 run（`Engine._run` 串行锁）；重负载一次一个；supervision 0.25.1；onnxruntime 互杀；
  Git Bash curl 中文 JSON 损坏（用 httpx）。
- 镜像：pip 清华/阿里云/官方；npm npmmirror/官方；GitHub ghfast；HF hf-mirror；Roboflow 代理 7890。

## 九、文档地图

| 想知道什么 | 看哪里 |
|---|---|
| 协作/任务状态 | `50-tasks/调度看板.md` + `派发状态.json` |
| 项目复审（bug/优化） | `62-project-audit-2026-10-28.md` |
| USB/OBS 摄像头 | `64-usb-virtual-camera-report.md` + `50-tasks/56-...md` |
| 打包分发 | `58-packaging-strategy.md` + `59-portable-install.md` |
| 模型评估 | `52-model-eval.md` |
| 发布 | `70-release-plan.md` + `发布核对报告.md` + `发布公告.md` |
| 真实摄像头 24h | `摄像头24h验收清单.md` + `tools/camera_soak.py` |
| 架构/API | `30-architecture.md` / `35-api-reference.md` |
