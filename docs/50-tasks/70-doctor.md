# 任务 70 · 便携包自检工具 doctor.py

> 背景：新机器上出问题（缺模型、缺 web/dist、端口占用、数据库损坏、缺 ffmpeg/go2rtc），现在要靠人工
> 逐项排查。做一个只读自检脚本，用户/支持人员一条命令拿到"哪项坏了、怎么修"。

## 目标

新增 `scripts/doctor.py`（只用标准库）：

```
python scripts/doctor.py [--root <包根/项目根>] [--port 8000] [--json] [--verbose]
```

检查项（每项输出 `PASS/WARN/FAIL` + 建议）：

1. **Python/运行时**：源模式检查 `sys.version_info >= (3,10)`；便携包模式检查
   `runtime/python.exe`、`runtime/pythonw.exe` 存在；
2. **程序文件**：`app/server/app/main.py`（或 `server/app/main.py`）、`web/dist/index.html`
   （或 `app/web/index.html`）、`scripts/portable_launcher.py`（便携包）；
3. **模型**：`models/yolo11n_ppe.onnx`（或 `m0-spike/yolo11n_ppe.onnx`）+ `*.classes.txt`；
   文件大小 > 1MB；classes 行数 > 0；
4. **媒体/工具**：`media/*.mp4` 至少 1 个；`tools/ffmpeg/ffmpeg.exe`、`tools/go2rtc.exe`
   （缺失 WARN，说明会影响 USB 枚举/实时画面）；
5. **配置**：`.env`/`config.env` 是否存在；`LS_ADMIN_TOKEN` 是否设置（只输出"已设置/未设置"，
   **绝不输出值**）；`LS_WEB_DIST`/模型路径是否指向存在的文件；
6. **数据库**：能打开、`PRAGMA integrity_check` = ok；输出 cameras/events 行数与文件大小；
   WAL 的 `-wal` 文件大小（存在属正常）；
7. **端口**：`--port` 可 bind（被占用 WARN/FAIL，说明可能是旧实例）；
8. **磁盘**：data 所在盘 free GB；低于 5GB WARN；
9. **写权限**：data/logs 目录可写（写临时文件后删）。

- `--json`：输出机器可读列表，便于支持脚本；
- 退出码：无 FAIL → 0；有 FAIL → 2；只有 WARN → 0（报告里说明）；
- 所有检查只读，不启动服务、不加载 ONNX、不打开摄像头。

## 集成

- `scripts/build_portable.py`：把 `doctor.py` 复制进便携包 `scripts/`（与 seed_demo 同处）；
- `docs/59-portable-install.md` 增加「启动失败先自检」小节：
  `runtime\python.exe scripts\doctor.py`；
- `docs/部署FAQ.md` 增加「doctor 输出怎么读」；
- `README.md` 故障排查段加一行链接。

## 测试 `server/tests/test_doctor.py`

- 用临时目录构造"最小可用包"（假 runtime/模型/classes/web dist/媒体/db）→ 全 PASS；
- 删模型 → FAIL 且建议含"模型"；删 web dist → FAIL；
- 数据库写入坏文件 → FAIL（integrity 失败）；
- `--json` 输出可被 `json.loads`，字段包含 name/status/detail；
- 测试不依赖真实大模型（用几 KB 假文件也可，只要大小检查可通过——若检查 >1MB，则在临时文件里写
  sparse/1MB 内容；或把最小大小做成参数供测试）。
- 注意：脚本在 `scripts/`，测试用 `importlib.util.spec_from_file_location` 导入，避免安装包。

## 约束

- 不自动修复、不删文件、不改注册表；不引入依赖；
- 不打印任何密钥/令牌/RTSP 密码；
- 便携包路径与源码路径都要支持（自动探测 `--root`）。

## 验收标准

- `python scripts/doctor.py` 在源码根实跑：FAIL 0，WARN 如实列出（如 go2rtc/ffmpeg 若缺失）；
- 对当前便携包解压目录实跑：PASS/FAIL 与预期一致（可临时改坏一项验证）；
- `pytest tests/ -q` 全绿（基线 91 + 新增）；
- 报告附实际输出（可截断）与退出码。

## 执行报告

**flash 2026-10-30 完成（pytest 151 passed，源码/便携双模式实跑 10 PASS / 0 FAIL）**

- **改动**：新增 `scripts/doctor.py`（纯标准库，9 大项 10 检查：运行时/程序文件/模型（>1MB+词表
  非空，MIN_MODEL_BYTES 可 monkeypatch）/演示媒体/ffmpeg+go2rtc/配置（**LS_ADMIN_TOKEN 只输出
  已设置/未设置**，相对路径按 server 目录解析）/数据库（integrity_check+行数+wal 大小）/端口
  bind/磁盘（<5GB WARN）/写权限（临时文件写删））；`--json` 机器可读；退出码 FAIL=2 否则 0；
  双布局自动探测（runtime/python.exe 存在=便携包，否则源码）。`build_portable.py` 已复制
  doctor.py 进包（与 seed_demo 同处）。
- **测试**：`test_doctor.py` 6 用例（importlib 加载；最小假包全 PASS / 删模型 FAIL+建议 / 删
  web dist FAIL / 坏 db FAIL+备份建议 / --json 可解析 / 小模型过不了阈值）。全量
  **pytest 151 passed**（145 + 6）。
- **实跑**：
  - 源码根：**10 PASS / 0 WARN / 0 FAIL**（模型 10.7MB、db integrity=ok cameras=4 events=40）；
  - 便携包目录（dist/portable/LabSentinel）：**10 PASS / 0 FAIL**（ffmpeg 扁平布局已识别）。
- **实跑暴露并修复的 3 个检查缺陷**：①mkstemp 句柄未关→Windows 删不掉误报写权限 FAIL；
  ②ffmpeg 只查 `tools/ffmpeg/ffmpeg.exe` 漏了便携包扁平布局 `tools/ffmpeg.exe`；
  ③config 相对路径按 CWD 解析→改为按 server 目录解析（与运行时一致）。
- **文档**：docs/59 加"启动失败先自检"小节；部署FAQ 加「doctor 输出怎么读」；README 文档表
  故障排查行加 doctor 提示。
- **遗留**：无。
