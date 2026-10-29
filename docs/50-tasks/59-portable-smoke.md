# 任务 59 · 便携包一键冒烟（zip → 启动 → 健康 → 停止）

> 背景：任务 46/55 的启动/停止验证靠手工 harness，每次重建包都要重复一遍，且容易污染真实
> `%LOCALAPPDATA%\LabSentinel`。需要一个**可重复、完全隔离、可 CI 化**的冒烟脚本，作为发布前的固定关卡。

## 目标

新增 `scripts/smoke_portable.py`（只用标准库）：

```
python scripts/smoke_portable.py --zip dist/LabSentinel-portable-*.zip [--port 8180] [--timeout 120] [--keep]
```

执行步骤（每步输出 PASS/FAIL 行，最后汇总，失败 exit 1）：

1. **准备隔离环境**：解压 zip 到 `%TEMP%\ls-smoke-<随机>`；创建沙箱目录 `sandbox\LocalAppData`，
   后续子进程 `LOCALAPPDATA` 指向它；预写 `sandbox\LocalAppData\LabSentinel\config.env`，内容仅
   `port=<指定端口>`（`first_run` 见 config 存在会跳过模板生成，`start_server` 会用显式环境变量补齐
   全部 `LS_*` 路径——实现前先读 `portable_launcher.py` 的 `first_run/start_server` 确认）。
2. **启动**：设 `LS_LAUNCHER_NO_BROWSER=1`；优先 `LabSentinel.exe`（无 exe 回退 `启动.bat`）；
   启动进程本身应很快退出，服务由 pythonw 托管。
3. **等就绪**：轮询沙箱 `logs\launcher.lock`（格式 `pid port`）最多 `--timeout` 秒；拿到端口后
   `GET /api/system` 必须 200，且 `backend` 非空、`version` 非空。
4. **前端**：`GET /` 200 且响应包含前端根节点标记（先看 `web/dist/index.html` 实际内容再断言，
   不要写死猜测字符串）；`GET /assets/` 下任一 js/css 资源 200（从 index.html 解析）。
5. **首启播种**：沙箱 `data\labsentinel.db` 存在；只读 sqlite 查 `cameras` 数量 ≥ 1（正常应为 4）。
6. **停止**：优先 `LabSentinel.exe --stop`（回退 `runtime\python.exe scripts\portable_launcher.py --stop`），
   返回码 0；最多等 15s 端口释放、`launcher.lock` 删除。
7. **隔离断言**：真实 `%LOCALAPPDATA%\LabSentinel` 的 mtime/文件数在本次运行前后不变（先记录再对比；
   不存在也算不变）。**绝不写真实目录**。
8. **清理**：删除临时解压目录；失败或 `--keep` 时保留并打印路径（含日志尾部 30 行）。

另加 `--dir <已解压目录>` 模式（跳过解压，便于调试）。

## 约束

- 不重建便携包、不改启动器/安装器逻辑。发现 bug：在报告写清复现步骤与建议，**不顺手改**（除非一行
  明显笔误，需在报告注明）。
- 不占 8000/5173；默认 8180，被占则失败提示（不自动漂移，便于断言）。
- 全程不打开浏览器、不弹出 MessageBox（`LS_LAUNCHER_NO_BROWSER=1`；若 30s 未就绪启动器会弹窗，
  所以 `--timeout` 必须小于 30s 或提前失败——实现时处理这个边界）。
- 测试结束必须清理进程：即使断言失败也要走停止+清理（try/finally）。

## 验收标准

- 对 `dist/LabSentinel-portable-20261029.zip` 实跑：PASS 全绿、退出码 0、临时目录已删；
- 反例验证：`--zip` 指向不存在的文件 → 清晰报错 exit 1；解压目录缺 `LabSentinel.exe` → 回退
  `启动.bat` 分支被走到（可在报告说明如何构造）；
- 运行后真实 `%LOCALAPPDATA%\LabSentinel` 未被修改；8000/8180 端口空闲、无残留 python/uvicorn；
- `python scripts/smoke_portable.py --help` 可用；不影响 `pytest` 基线（91 passed）。
- 文档：`scripts/README.md` 与 `docs/59-portable-install.md` 增加冒烟命令与输出示例。

## 执行报告

**flash 2026-10-30 完成（正例 8/8 PASS ×2 路径 + 反例全过，未改启动器/安装器）**

- **交付**：`scripts/smoke_portable.py`（纯标准库，8 步 PASS/FAIL，try/finally 兜底杀残留）；
  docs/59 与 scripts/README.md 已加命令与说明。未改 portable_launcher/install_portable/build_portable。
- **正例实跑**（dist/LabSentinel-portable-20261029.zip，端口 8180）：
  `8/8 PASS`，exit 0——隔离解压→`LabSentinel.exe` 启动→锁出现→`/api/system 200 backend=directml
  version=0.1.0-m1`→首页根标记命中+`/assets/index-DHSDXJpu.js 200`→播种 cameras=4→`--stop` rc=0
  端口释放锁删除→**真实 %LOCALAPPDATA%\LabSentinel 文件数/mtime 前后不变 (59, ...)→临时目录已删**。
  stop_cmd 重构后复跑一遍仍 8/8。
- **回退分支实测**：解压真实 zip 剥除 `LabSentinel.exe` 后 `--dir` 模式 → 入口=启动.bat，
  同样 8/8 PASS exit 0。
- **反例**：`--zip 不存在` → `[失败] zip 不存在` exit 1；缺参数 → argparse 用法报错 rc=2。
- **发现的 bug（修复于冒烟脚本自身，未动产品代码）**：初版 `stop_cmd` 固定指向 LabSentinel.exe，
  回退分支（无 exe）停止时 FileNotFoundError 直接崩溃——已改为随入口切换停止命令并复测。
  产品代码无 bug 发现。
- **边界处理**：`--timeout` ≥30 钳到 25（避开启动器 30s MessageBox）；端口被占不漂移直接失败。
- **pytest**：无服务端代码改动，基线不受影响（当前 **95 passed**，含任务 58 新增 4 例）。
- **遗留**：无。CI 化时把本脚本接进 portable.yml 的构建后步骤即可（记入 52 报告的后续优化）。
