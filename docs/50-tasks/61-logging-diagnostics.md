# 任务 61 · 服务端文件日志轮转 + 一键诊断包

> 背景：便携版服务日志目前只有启动器把 uvicorn stdout 重定向到 `logs/server-<日期>.log`，没有轮转、
> 没有统一格式；用户报障时只能远程指导"去哪个目录找哪个文件"。本任务补服务端自身日志落盘（带轮转）
> 和一个脱敏诊断包，降低支持成本。

## 目标

### 1. `server/app/logging_setup.py`

- `setup_file_logging() -> Path | None`：幂等（重复调用不重复加 handler）。
  - 文件：`settings.data_dir / "logs" / "server.log"`；
  - `RotatingFileHandler(maxBytes=5*1024*1024, backupCount=3, encoding="utf-8")`；
  - 格式：`%(asctime)s %(levelname)s %(name)s %(message)s`；
  - 级别默认 INFO，可用 `LS_LOG_LEVEL` 覆盖；
  - `LS_LOG_FILE=0`（或 `false`）关闭文件日志，返回 None；
  - **与 uvicorn 自带 handler 共存**：不删不改 root 现有 handler，只追加文件 handler；避免同一条日志
    被重复写（文件 handler 单独挂 root，uvicorn 的 console handler 保持）。
- 在 `server/app/main.py` 模块导入阶段或 lifespan 启动时调用一次（导入阶段更早，能捕获启动错误；
  注意不要 import 时就创建目录导致测试污染——测试可用临时 `LS_DATA_DIR`）。

### 2. `GET /api/diagnostics/summary`（读保护口径同其它 GET）

返回（全部为可公开的聚合信息，**不含事件详情/截图/密钥**）：

```json
{
  "version": "0.1.0-m1",
  "backend": "directml",
  "uptime_s": 123,
  "counts": {"cameras":4,"zones":4,"rules":8,"events":120,"alert_deliveries":3},
  "disk": {"data_dir":"...","free_bytes":123,"total_bytes":456},
  "logs": {"file":"...server.log","size_bytes":123,"rotated":["server.log.1"]},
  "recent_errors": ["2026-10-30 ... ERROR ..."]
}
```

- `recent_errors`：从进程内 ring buffer（自定义 `logging.Handler` 保留最近 50 条 WARNING+，只取 ERROR
  最近 20 条）读取；没有则空数组。ring buffer 不写库、不落盘。
- 所有路径只返回字符串；磁盘用 `shutil.disk_usage`；计数用 `Store` 现有方法或只读 SQL。

### 3. `scripts/collect_diagnostics.py`

```
python scripts/collect_diagnostics.py --out diag.zip [--data-dir server/data] [--log-lines 500]
```

打包内容（zip 内 `README.txt` 说明每项用途）：

- `system.txt`：平台、Python 版本、CPU 数、内存（`os.cpu_count`；内存可用 `ctypes` 或省略，不引 psutil）、
  版本、git commit（可选）；
- `config.txt`：当前生效配置的**脱敏**键值——`LS_ADMIN_TOKEN`/`*PASSWORD*`/`*SECRET*`/`*TOKEN*`/
  `*KEY*` 的值一律 `***`；URL 中的 `user:pass@` 替换为 `***:***@`；只输出 `LS_` 前缀键；
- `counts.txt`：sqlite 只读打开 db，输出各表行数（表不存在写 0/跳过）；
- `logs/server.log.tail.txt`：日志最后 N 行（文件不存在写说明）；
- `cameras.txt`：摄像头 id/name/type/status（**不含 RTSP 地址中的账号密码**，source 只输出
  协议+主机或 `***`）。
- **绝不包含**：数据库整库、截图、视频、`.env` 原文件、人脸数据。
- 输出 zip 后打印条目数与大小；`--out` 已存在默认拒绝（`--force` 覆盖）。

### 4. 测试 `server/tests/test_logging_diag.py`

- `setup_file_logging`：临时 `LS_DATA_DIR` 下生成 `logs/server.log`；重复调用 handler 数不增加；
  `LS_LOG_FILE=0` 返回 None 且不建文件；`maxBytes/backupCount` 参数正确（可读 handler 属性）；
- 脱敏函数：token/password/secret/key、URL 凭证、普通键不受影响；
- `diagnostics/summary`：TestClient 200、字段齐全、无敏感串、logging ring 能捕获一条 ERROR；
- `collect_diagnostics.py`：对临时 data 目录（小 db + 假日志）产 zip，断言条目齐全、无敏感值、
  db 内容未被整库打包；
- 测试后清理临时目录；不要把测试日志写进真实 `server/data`。

### 5. 文档

- `docs/部署FAQ.md` 增加「日志在哪 / 怎么生成诊断包 / 轮转策略」；
- `docs/59-portable-install.md` 便携版日志路径一节补充 `server.log` 与诊断包命令。

## 约束

- 不引入新依赖（标准库 logging/shutil/sqlite3/zipfile）；
- 不改业务日志文案与级别；不动 `theme.css/style.css/App.vue/main.js`；
- 诊断包默认输出到当前目录或 `--out` 指定处，测试用临时目录；
- 不把真实 `server/data/labsentinel.db` 打进包（只计数）。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；8001 临时实例实跑 `/api/diagnostics/summary` 200；
- 文件日志实际生成；把 `maxBytes` 临时调小（如 2KB）验证产生 `.1` 轮转文件（验证后恢复）；
- 诊断 zip 可解压、条目与 README 一致、grep 不到真实 token/密码；
- 文档命令可直接复制执行。

## 执行报告

**flash 2026-10-30 完成（pytest 109 passed，端点/文件日志/诊断包实跑全过）**

- **改动文件**：新增 `server/app/logging_setup.py`（幂等文件日志 5MB×3+RingHandler 200 条 WARNING+、
  LS_LOG_LEVEL/LS_LOG_FILE、uptime）、`server/app/diagnostics.py`（summary 聚合）、
  `scripts/collect_diagnostics.py`（6 条目脱敏 zip）、`server/tests/conftest.py`（测试会话默认
  LS_LOG_FILE=0 防污染）；`api.py` 加 `GET /api/diagnostics/summary`（require_read 口径）+ 抽出
  `_system_version()` 单一来源；`main.py` 导入阶段调 `setup_file_logging()`（失败静默）。
- **测试**：`test_logging_diag.py` 7 用例（幂等与参数/关闭开关/小 maxBytes 轮转产生 .1/ring 捕获
  ERROR/脱敏函数全类/summary 端点字段与 ring/诊断包条目+脱敏+拒绝覆盖+不整库打包）。全量
  **pytest 109 passed**（102 + 7）。
- **实跑**（8001 临时实例 + 敏感 env LS_ADMIN_TOKEN）：summary 200（version/backend/uptime_s/
  counts 五表计数/disk/logs/recent_errors 齐全）；`logs/server.log` 实际生成；诊断包 6 条目 1.9KB，
  **zip 全文 grep 不到真实 token** ✓；轮转用例以 2KB maxBytes 实际产生 server.log.1。
- **如实记录**：uvicorn 的 access 日志不进 server.log（其 logger propagate=False 且约束要求不删
  不改 uvicorn handler）；业务模块（engine/pipeline 等）日志走 root 正常落盘。
- **文档**：部署FAQ 加「日志在哪/诊断包/轮转策略」；docs/59 数据位置节补 server.log 轮转说明与
  便携版诊断包命令。
- **遗留**：无。
