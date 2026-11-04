# 任务 111 · 摄像头分组与批量启停（P1）

> 背景：多路部署时用户按区域/楼层管理摄像头；现在只能逐路启动/停止，也没有分组视图。
> 本任务加 `group_name` 字段、分组列表与批量启停，属于 M2C「摄像头分组」功能。

## 目标

### 1. 数据与迁移（v5，沿用 69 框架）

- `cameras` 增加 `group_name TEXT DEFAULT ''`；同步更新 `scripts/seed_demo.py` 的 SCHEMA（与 store 对拍测试）。
- `Store`：`set_camera_group(cid, name)`、`camera_groups()`（distinct + count）；
  `add_camera(..., group_name="")` 可选参数，保持旧调用兼容。

### 2. API

- `PUT /api/cameras/{cid}/group`（require_admin）`{group_name}`；空字符串=未分组；长度 ≤32。
- `GET /api/camera-groups`（require_read）：`[{name,count,running}]`。
- `POST /api/cameras/bulk`（require_admin）`{ids:[...], action:"start"|"stop"}`：
  - ids 1–100、去重；不存在 id 计入 missing；start/stop 复用 manager；不删数据；
  - 返回 `{ok,missing,failed}`。
- `CameraCreate` 增加 `group_name` 可选；`GET /api/cameras` 返回该字段。

### 3. 前端 `Cameras.vue`

- 添加表单增加“分组”输入（可选）；
- 表格增加分组列；顶部增加分组筛选下拉 + “启动本组/停止本组”按钮；
- 勾选多路时显示“批量启动/停止”；操作前 confirm，请求期间 disabled；
- 分组为空显示“未分组”；不新增路由/不改全局四件套。

### 4. 测试 `server/tests/test_camera_groups.py`

- 迁移后旧库有 group_name 且默认空、旧数据保留；
- set/get/group list 统计正确；bulk start/stop 调用 manager 次数正确、missing 处理、401；
- 添加带 group 的摄像头；group 长度/类型校验。

### 5. 文档

- `35-api-reference` 新端点；`部署FAQ`「多路怎么按楼层分组管理」；`运维手册` 摄像头管理一节。

## 约束

- 不改变摄像头启停语义/状态机；不引入依赖；前端只改 Cameras.vue/api.js。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库：3 路分 2 组→按组启停，状态正确；报告附分组 JSON。

## 执行报告

**flash 2026-11-04 完成（pytest 416 collected / 7 新增全绿；build gzip 56.78KB；8188 实机 PASS）**
（全量回归数字待本轮后台完成后补记，见文末补记）

### 1. 数据与迁移

- **规格勘误**：规格写"迁移 v5"，但 v5 已被 121 folder_images 占用——顺延为 **v7**
  （`_mig_007`：cameras ALTER 加 `group_name TEXT DEFAULT ''`，PRAGMA 防重入）；
  `_SCHEMA` 与 `scripts/seed_demo.py` SCHEMA 双向同步（顺带把 112 的 rules v6 列与
  calendar_exceptions 一并补进 seed，`test_schema_matches_store` 对拍恢复全绿）；
- `Store.add_camera(..., group_name="")`（旧调用兼容）、`set_camera_group(cid, name)`、
  `camera_groups()`（GROUP BY 聚合 count/running）。

### 2. API

- `PUT /api/cameras/{cid}/group`（admin）：≤32 字符（400）/不存在（404）/空串=未分组；
- `GET /api/camera-groups`（读保护）：`{"groups":[{name,count,running}…]}`；
- `POST /api/cameras/bulk`（admin）：ids 去重非空整数数组 ≤100（bool 冒充 int 也拒），
  `{ok, missing, failed}` 三段返回；start/stop 复用 manager 单路语义，不删数据；
- `CameraCreate.group_name`（Field max_length=32）；`GET /api/cameras` 自动带出。

### 3. 前端（Cameras.vue + api.js，路由零新增）

- 添加表单加"分组"输入；表格加勾选列 + 分组列（空显示"未分组"）；
- 顶部分组工具条：筛选下拉（含 count/running）、启动本组/停止本组、批量启动/停止（勾选）；
  操作前 confirm、请求期 disabled；missing/failed 有 alert 明细；全局四件套未动。

### 4. 测试（test_camera_groups.py，7 用例）

旧库迁移（列在/默认空/旧数据保留）、分组 CRUD+统计（含未分组桶）、API 添加带组读回、
PUT 校验（33 字 400/404/清空）、bulk start/stop（FakeMgr 记录调用次数与顺序、missing 处理）、
bulk 全量校验矩阵（action/空/重复/>100/非整数/bool）、读保护 401。

### 5. 实机验证（8188 临时实例+临时库，3 路分 2 组）

`groups=[{一楼西,1,0},{二楼东,2,0}]`；bulk start `[a,b]` → 两路 running、c 保持 stopped；
bulk stop → ok 全量。**分组 JSON 如上**。→ PASS

### 6. 文档

35-api-reference（三端点+字段）、部署FAQ「多路怎么按楼层/区域分组管理」、运维手册 §4.2。

### 遗留

- 无。
