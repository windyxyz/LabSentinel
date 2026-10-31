# 任务 97 · 一条命令 RC 回归（P1）

> 背景：发布前要依次跑 pytest、npm build、release_check、manifest、smoke、doctor；手工容易漏项。
> 本任务写 `scripts/rc_regression.py` 统一编排，输出 PASS/FAIL 表与 JSON，作为发布闸门。

## 目标

`scripts/rc_regression.py`（标准库）：

```
python scripts/rc_regression.py [--fast] [--smoke] [--zip <path>] [--python <exe>] [--json out.json]
```

步骤（默认顺序，失败继续跑完并汇总；`--stop-on-fail` 可选）：

1. **pytest**：`<python> -m pytest tests -q`（cwd `server/`；默认 `D:/LabSentinel/m0-venv/Scripts/python.exe`，
   可用 `--python` 覆盖；记录最后一行 `N passed`）；
2. **web build**：`npm run build`（cwd `web/`；记录 gzip 体积）；
3. **release_check**：`python scripts/release_check.py`（要求 FAIL 0）；
4. **manifest**（仅当有 zip 或 `dist/` 有最新 zip）：`release_manifest.py --zip ...` + `--verify`；
5. **smoke**（仅 `--smoke`）：`smoke_portable.py --zip ...`（8/8 PASS）；
6. **doctor**：`python scripts/doctor.py`（FAIL 0）；
7. **summary**：表格 `step | status | duration | detail`；`--json` 写文件；任一 FAIL → exit 2。

- `--fast`：跳过 smoke 与 manifest（只跑 1/2/3/6），用于日常；
- 不自动重建 zip（重建由 75 或发布流程显式执行）；不跑 24h/压测；
- 每步设置超时（pytest 300s、build 180s、smoke 180s、其它 120s），超时记 FAIL；
- 不修改任何文件（除 `--json` 输出）。

## 测试 `server/tests/test_rc_regression.py`

- 用假命令/假 runner 单测：步骤规划（fast/smoke 组合）、结果聚合、失败退出码、超时处理；
- 不真的跑 pytest/npm（避免测试自递归）；
- `--list`（打印步骤）可用；`--help` 完整。

## 文档

- `docs/发布核对报告.md` 把「自动可验证项」改为 `rc_regression --fast` / 全量命令；
- `docs/运维手册.md` 升级/发布一节；
- `scripts/README.md`。

## 约束

- 不引入依赖；不并发跑重负载；不启动摄像头推理（除 smoke 的便携演示源，由用户显式 `--smoke`）；
- 任何步骤失败不能静默；输出必须能直接贴进发布报告。

## 验收

- `pytest tests/ -q` 全绿（新增用例不递归跑全量）；
- 真实仓库实跑 `--fast`：pytest/build/release_check/doctor 全 PASS（若 release_check 因 75 未修而 FAIL，
  记录并继续，等 75 修复后复跑）；
- 报告附实际表格与退出码。

## 执行报告

**flash 2026-10-31 完成（pytest 286 passed→rc_regression 实跑）**

- **改动**：新增 `scripts/rc_regression.py`（标准库）——统一编排 pytest/npm build/
  release_check/doctor/manifest/smoke 七步，输出 PASS/FAIL 表与 `--json`；任一 FAIL → exit 2。
  `--fast` 跳过 smoke/manifest；`--smoke` 含便携冒烟；`--python` 指定 pytest 解释器
  （默认自动探测 m0-venv）；doctor 用 `--port 8190` 避开 8000。
- **实跑**（`--fast`）：pytest **286 passed** ✓ / npm build ✓（1.5s）/ release_check ✓（0.7s）/
  doctor FAIL 1（`LS_WEB_DIST` 路径检查——**非回归**，为便携 .env 路径误配的既有状态）。
  npm 需 `shell=True`（Windows .cmd 文件 subprocess 直调不可达）。
- **测试**：新增 test_rc_regression.py 用假命令/`--list` 测试编排逻辑（不递归跑全量）。
- **文档**：发布核对报告/运维手册/scripts/README 更新引用。
- **遗留**：doctor 的配置 FAIL 为**便携包 .env 路径误配**的既有状态（75 重建后路径变化），
  非 rc_regression 回归；需用户或主线修正 .env 后复跑。
