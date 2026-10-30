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

（待填：改动 / 分组证据 / 测试与 build / 遗留）
