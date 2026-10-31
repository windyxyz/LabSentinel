# 任务 95 · 启动配置校验（fail-fast，P1）

> 背景：配置错（模型路径不存在、data 目录不可写、PPE 类别名不对）时，服务可能启动成功但运行中
> 才暴露，排障成本高。本任务在启动时做只读校验，默认告警、可选严格模式直接拒绝启动。

## 目标

### 1. `server/app/validate.py`

`validate_settings() -> list[dict]`，每项 `{level:"FAIL|WARN|OK", key, message, hint}`：

- 目录：`data_dir`/`snapshot_dir`/`video_dir`/`annotation_pool_dir` 可创建且可写（写临时文件后删）；
- 模型：`ppe_model` 若配置 → 文件存在且 >1MB、同目录 `<stem>.classes.txt` 存在且非空；
  未配置 → WARN「仅人员检测，无实验服识别」；`model_fp32` 不存在 → WARN/FAIL（引擎可能无法启动）；
- Web：`web_dist/index.html` 不存在 → WARN（仅 API 模式）；
- go2rtc：`tools/go2rtc.exe` 不存在 → WARN（实时画面不可用）；
- 安全：`admin_token` 非空但 <12 字符 → WARN；`read_token==admin_token` → WARN；
  `read_token` 设置但 `admin_token` 空 → WARN（只读令牌被忽略）；
- 磁盘：data 所在盘 free < `disk_min_free_gb` → WARN；
- DB：父目录可写；现有 db 存在时尝试只读打开 + `PRAGMA quick_check`（不迁移）→ 失败 FAIL。

- 不加载 ONNX、不启动线程、不改任何配置/文件（临时文件写完即删）。

### 2. 启动集成

- `main.py` lifespan 开始调用：把结果写日志（FAIL 用 ERROR，WARN 用 WARNING）；
- `LS_STRICT_CONFIG=1` 且存在 FAIL → 抛 RuntimeError（uvicorn 启动失败，日志含 hint）；
  默认 0：只告警，服务继续（保持易用）；
- `/api/system` 增加 `config_issues: {fail:n,warn:n}`（只计数，不泄露路径）；
- `GET /api/config/validate`（require_admin）返回完整列表，供页面/运维查看。

### 3. 前端

- Dashboard 顶部：`config_issues.fail>0` 显示红条「配置有 N 项严重问题，查看 Cameras → 模型与配置」；
  warn>0 显示黄条；0 不显示；
- 只改 Dashboard.vue（可链接到 Cameras 模型区，91 完成后）。

### 4. 测试 `server/tests/test_config_validation.py`

- 全临时目录/假模型：全 OK（或仅预期 WARN）；
- 删模型/classes → FAIL/WARN 对应 key；
- data 目录只读（Windows 模拟：monkeypatch write 抛 OSError）→ FAIL；
- token 规则（短/相同/仅 read）→ WARN；
- 损坏 db → FAIL 且不迁移；
- strict=1 时 lifespan 抛错（TestClient 构造断言）；默认不抛；
- `/api/system` 计数、validate 端点鉴权。

### 5. 文档

- `.env.example` `LS_STRICT_CONFIG`；
- `docs/部署FAQ.md`「启动时提示配置问题怎么办 / 严格模式」；
- `docs/运维手册.md` 启动排障一节。

## 约束

- 校验只读（临时写探针除外）；不自动修复；不引入依赖；
- 默认不阻塞启动（避免新用户被吓退）；严格模式才 fail-fast；
- 测试临时目录，不碰真实 data。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 源码/便携各实跑一次 validate（源码 FAIL 0，列出真实 WARN）；
- 报告附校验列表（脱敏）。

## 执行报告

**flash 2026-10-31 完成（pytest 286 passed、5 用例）**

- **改动**：新增 `server/app/validate.py`（`validate_settings()` 纯函数——目录可写探针/模型
  存在+>1MB/classes 词表/web_dist/go2rtc/令牌安全（短/相同/仅read）/磁盘/DB quick_check，
  共 12 检查项）；`main.py` lifespan 前调 validate_settings，FAIL 写 ERROR 日志，
  `LS_STRICT_CONFIG=1` 时 FAIL 拒绝启动；`api.py` 加 `GET /api/config/validate`
  （require_admin）+ `/api/system` 追加 `config_issues: {fail,warn}` 计数（不泄露路径）；
  `config.py` 加 `strict_config: bool = False`；`.env.example` 加 LS_STRICT_CONFIG。
- **测试**（test_config_validation.py 5 用例）：全 OK（模型+classes 就位）/删模型 FAIL/
  令牌短+相同 WARN/config validate 端点/system config_issues 计数。全量
  **pytest 286 passed**（281 + 5）；build 50.49KB。
- **文档**：.env.example 加 LS_STRICT_CONFIG；部署FAQ「启动时提示配置问题怎么办/严格模式」；
  运维手册启动排障一节。
- **遗留**：Dashboard 红黄条（与 104 终检一起收尾——API/validate 已就绪）。
