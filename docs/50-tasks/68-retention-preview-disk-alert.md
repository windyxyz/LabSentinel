# 任务 68 · 保留策略预演 + 磁盘水位预警

> 背景：`retention.run_cleanup()` 会直接删过期事件和文件；用户在执行前无法知道"会删多少、释放多少"。
> 磁盘不足时也只有后台紧急清理，没有提前预警。本任务增加**只读预演**和**前端水位提示**，
> 让清理动作可预期。

## 目标

### 1. `retention.py` 增加预演函数

- `preview_cleanup(store) -> dict`：与 `run_cleanup` 使用**同一套时间阈值/路径安全判断**，但
  **绝不删除任何东西**；返回：
  ```json
  {"cutoff":"2026-05-03T...","events":{"count":12,"bytes":3456789},
   "pool":{"files":3,"bytes":123456},"disk_free_gb":8.4,
   "estimated_free_gb":8.43,"would_emergency_clean":false}
  ```
- 统计事件对应的截图/视频文件大小时做路径安全校验（复用 `_safe_within`），文件缺失跳过；
- `run_cleanup` 内部可复用预演逻辑（避免两套阈值漂移），但**行为不变**。

### 2. API

- `GET /api/maintenance/preview`（读保护口径同其它 GET）：调用 `preview_cleanup`；
- `GET /api/maintenance/status` 追加：
  - `disk_total_gb`、`disk_free_gb`、`disk_free_pct`；
  - `data_bytes`（data_dir 递归大小，限制遍历深度/文件数，避免大目录卡 API；超过 2 万文件可只统计
    db+snapshots+videos+pool 四目录）；
  - `low_disk`（free < `settings.disk_min_free_gb`）；
  - `forecast_days`（粗略：近 7 天新增事件文件字节/天 → free/日增；数据不足返回 null，
    **明确标注为估算**）。
- 可选：`POST /api/maintenance/preview` 不需要；不要新增删除类端点。

### 3. 前端 `web/src/views/Dashboard.vue`

- 在卡片区增加一张「磁盘」卡：显示 `free_gb / total_gb`（保留 1 位小数）；
- `low_disk=true` 时卡片变红并显示「磁盘不足，将触发紧急清理」；
- 下方（或卡片 title）显示「按当前增速约 N 天」；null 不显示；
- 不引入图表库；只改本组件（可改 `api.js` 加 `maintenanceStatus/preview`）。

### 4. 测试 `server/tests/test_retention_preview.py`

- 临时 data 目录造 3 条过期事件（含假截图/视频）+ 2 条未过期事件 + 误报池文件；
- `preview_cleanup` 返回的 count/bytes 正确，且**调用后文件与行都还在**；
- `run_cleanup` 仍按原语义删除，返回统计与预演一致（允许因实际删除顺序导致 bytes 细微差异，
  但 count 必须一致）；
- 路径穿越/文件缺失不崩；`/api/maintenance/preview` 200 与字段齐全；
- 真实 `server/data` 不参与测试。

### 5. 文档

- `docs/部署FAQ.md` 增加「清理前怎么预演 / 磁盘预警阈值怎么调」；
- `docs/35-api-reference.md` 增加 preview 端点与 status 新字段；
- `docs/59-portable-install.md` 数据目录一节补充磁盘预警说明。

## 约束

- 预演必须**只读**：不能删文件、不能写库、不能触发 checkpoint 以外的写操作；
- 不改变 `run_cleanup` 的删除顺序/阈值语义；不引入依赖；
- 数据目录大小统计要有超时/上限保护，不能在几十万文件时卡死 API。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；`npm run build` 通过；
- 8001 临时实例调用 preview/status 各一次，返回与临时数据一致；
- Dashboard 在 `low_disk=true` 时有可见红色提示（可用临时把阈值调大验证，验证后恢复）。

## 执行报告

**flash 2026-10-30 完成（pytest 139 passed、build 45.27KB、8001 实跑预演/水位/红卡全过）**

- **改动**：`retention.py` 加 `preview_cleanup(store)`（与 run_cleanup 同一套 cutoff/`_safe_within`
  路径判断/缺失文件跳过语义，分批 SELECT 只读统计，**零删除**；run_cleanup 本体未动——复用预演
  需重构删除循环，为保"行为不变"约束选择不并，避免两套阈值漂移靠共享常量 `_BATCH`/settings 阈值
  达成）；`api.py` 加 `GET /api/maintenance/preview`；`maintenance/status` 追加 disk_total_gb/
  disk_free_pct/data_bytes（os.walk 超 2 万文件退化为 db+三目录统计，`data_bytes_capped` 标注）/
  low_disk/forecast_days（近 7 天事件关联文件日均字节估算，带"估算"标注，异常不阻塞端点）。
  `Dashboard.vue` 加「磁盘」卡（free/total 1 位小数；low_disk 红卡+"将触发紧急清理"；
  forecast"按当前增速约 N 天（估算）" title 提示；轮询随 reload 每 5s 刷新）。
- **测试**：`test_retention_preview.py` 4 用例（预演 count/bytes 正确且**文件与行原样在**/实删
  deleted_events 与预演一致且池文件语义正确/缺失文件与 `../../` 穿越路径不崩且穿越不计字节/
  status+preview 端点字段齐全）。全量 **pytest 139 passed**（135 + 4）。
  （坑：TestClient lifespan 会启动 retention 后台线程当场删掉过期数据——测试 monkeypatch
  start_scheduler 关调度。）
- **8001 实跑**：preview `{"events":{"count":1,"bytes":2048},...,"would_emergency_clean":true}`
  （LS_DISK_MIN_FREE_GB=99999 强制低水位）；status `disk_free_gb=145.44 disk_total_gb=828.06
  low_disk=true forecast_days=994193088`（日增仅 1KB 时的天文数字估算，真实负载下有意义）；
  playwright 确认磁盘卡渲染 + 红色"磁盘不足，将触发紧急清理" ✓（截图 D:\tmp\ls68_disk_card.png）。
- **build**：通过，gzip **45.27KB**（44.88 → +0.39KB）。
- **文档**：35-api-reference 加 §8.2 维护/预演（原 8.2 顺延 8.3）；部署FAQ 加「清理前怎么预演/
  磁盘预警阈值怎么调」；docs/59 数据目录节补磁盘水位说明。
- **遗留**：无。
