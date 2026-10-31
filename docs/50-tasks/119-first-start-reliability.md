# 任务 119 · 便携首次启动可靠性 + doctor 源模式（P1）

> 来源：75 主线复验实跑（2026-10-31）。新 zip 冷启动约 29s 才响应 `/api/system`，启动器 30s 到点
> 弹窗判负；另发现便携 `go2rtc.yaml` 路径错误、doctor 源模式误读便携配置。三项一起修，修完重建。

## 1. 启动器就绪等待（根因：30s 太紧）

- `scripts/portable_launcher.py`：
  - 就绪等待 `30s` → **`LS_READY_TIMEOUT_S`（默认 60）**；每 5s 写一条“等待中 Ns”日志；
  - 自动化/无人值守（`LS_LAUNCHER_NO_BROWSER=1` 或新增 `LS_NO_MESSAGEBOX=1`）**不弹 MessageBox**，
    改为写 ERROR 日志并返回非 0；
  - 超时后仍清理子进程/锁（避免残留 uvicorn）。
- `scripts/smoke_portable.py`：等待窗口必须**大于**启动器超时（如 launcher 60s → smoke 90s），
  且不要用 25s 钳制；失败时打印 server/launcher 日志尾部。
- 验证：清空 `%TEMP%` 解压目录 + 冷启动（可先重启/清文件缓存无法精确模拟，至少连续 3 次冷启动）；
  记录从进程启动到 `/api/system` 200 的耗时，要求 <60s 且 smoke 8/8。

## 2. 便携 go2rtc 配置路径

- 日志：`[go2rtc] 配置已写入 …\LabSentinel\app\tools\go2rtc.yaml`；实际二进制在包根
  `tools\go2rtc.exe`（`app/tools` 不是包内工具目录）。
- 修复：便携启动器显式设置 `LS_GO2RTC_YAML=<包根>/tools/go2rtc.yaml`（或 `go2rtc.py` 按
  `runtime/python.exe` 推导包根）；确认 go2rtc.exe 定位同样指向包根，不写 `app/tools`。
- 验证：解压新包启动后，文件确实在 `tools/go2rtc.yaml`；`/api/cameras/{id}/webrtc` 正常返回；
  如本机 go2rtc 未运行，至少配置路径正确、无 app/tools 残留。

## 3. doctor 源模式配置选择

- `scripts/doctor.py` 当前 `first(data_root/config.env, root/server/.env)`：源模式会优先读
  `%LOCALAPPDATA%\LabSentinel\config.env`（旧便携配置），把 `LS_MODEL_FP32` 判成不存在 → 假 FAIL。
- 修复：`mode == "portable"` → `data_root/config.env`；源模式 → `root/server/.env`（不存在再回退）。
- 验证：源根 `python scripts/doctor.py` FAIL 0；包内 doctor FAIL 0；`rc_regression --fast` doctor PASS。

## 4. 复验链

1. `pytest tests/ -q`（基线 286；不回归）；
2. `npm run build`；
3. `python scripts/build_portable.py --zip`；
4. `release_manifest --zip <新 zip>` + `--verify`；
5. `smoke_portable --zip <新 zip>`（8/8，冷启动 <60s）；
6. 解压包内 `doctor.py --root <根>`（FAIL 0）；
7. `rc_regression.py --fast`（doctor 不再 FAIL）；
8. 记录新 zip 名/大小/sha256，更新 `docs/发布核对报告.md`。

## 约束

- 不改业务逻辑；只改启动器/doctor/smoke/路径；不引入依赖；
- 不改 VM/不跑推理压测；VM 验收在 118 做；
- 不 commit/不发布，报告写证据。

## 执行报告

（待填：冷启动耗时 / go2rtc 路径 / doctor 源模式 / smoke / rc_regression / 遗留）
