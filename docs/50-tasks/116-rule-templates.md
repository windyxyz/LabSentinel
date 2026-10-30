# 任务 116 · 规则模板库（P2）

> 背景：多实验室/多摄像头常用同一套规则（如“进入未穿实验服 L3、冷却 60s、仅工作日 8:00-18:00”）。
> 任务 80 支持从一路复制到多路，但换实验室/新装环境仍要重配；本任务提供**命名模板**保存/套用/导入导出。

## 目标

### 1. 数据（迁移 v7）

```sql
CREATE TABLE rule_templates(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT UNIQUE NOT NULL,
  config_json TEXT NOT NULL,   -- [{behavior,level,cooldown_s,time_start,time_end,threshold,days_of_week,holiday_only}]
  created_ts TEXT, updated_ts TEXT
);
```
- 模板只存规则，不含摄像头/区域 ID；套用时 zone 由用户选择（可为空=全画幅）。

### 2. API（require_admin 写，读按口径）

- `GET /api/rule-templates`：列表（含规则数/更新时间）。
- `POST /api/rule-templates` `{name, rules:[...]}`：校验 behavior/level/时间格式/星期；
  name 重复 409（或 upsert 由 `?replace=1` 控制，定义并测试）。
- `PUT /api/rule-templates/{id}`、`DELETE`。
- `POST /api/rule-templates/{id}/apply` `{camera_ids:[...], zone_id?:int}`：
  - 对每个目标摄像头创建规则；zone_id 只允许属于该摄像头（否则 400）；
  - 已存在同 behavior+zone+时间的规则跳过；返回 created/skipped；
  - 不自动启动摄像头。
- `GET /api/rule-templates/{id}/export` / `POST /api/rule-templates/import`（JSON，同 79 风格）可选。

### 3. 前端 `Zones.vue`

- 规则区顶部「模板」下拉 + 「套用到当前摄像头」；「保存当前规则为模板」按钮（输入名称）；
- 模板管理折叠区：列表/删除/查看规则数；
- 套用后刷新规则列表；失败显示 detail；不新增路由。

### 4. 测试 `server/tests/test_rule_templates.py`

- 创建/重复名/非法 rule/鉴权；套用到 2 路 → 规则数正确、zone 校验；
- 已存在规则跳过；模板不含 camera_id；删除后套用 404；
- 迁移旧库模板表为空、不影响规则。

### 5. 文档

- `35-api-reference`；`部署FAQ`「怎么复用一套规则到多个实验室」；`运维手册` 规则配置。

## 约束

- 模板只存规则，不存摄像头/区域绑定；不复制事件；不引入依赖；
- 与 80 的“复制现有摄像头配置”并存，不合并（一个是即时复制，一个是可复用模板）。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库：保存 2 条规则模板→套用到 2 路→每路 2 条规则；报告附 JSON。

## 执行报告

（待填：改动 / 套用证据 / 测试与 build / 遗留）
