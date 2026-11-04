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

**flash 2026-11-04 完成（pytest 436 passed = 431+5 新增；build gzip 58.86KB；8188 实机 PASS）**

### 1. 数据（**迁移 v8**——规格写 v7 已被 111 占用，顺延）

- `_mig_008`：`rule_templates(id, name UNIQUE, config_json, created_ts, updated_ts)`；
  `_SCHEMA` 基线同步；Store CRUD：`rule_templates/get_rule_template/add_rule_template
  （重名返回 None）/update_rule_template（0 不存在/-1 名冲突/1 成功）/delete_rule_template`。

### 2. API（写 require_admin，读按口径）

- `GET` 列表带 rules_count（解析 config_json）；`POST` `{name ≤64, rules 1-50 条}`——
  逐条走 `RuleCreate` 校验+behavior/HH:MM/星期集合补充校验（非法 400），**重名 409**
  （定义：无 upsert，测试覆盖）；`PUT`（404/409）、`DELETE`（404）；
- `POST /{id}/apply` `{camera_ids ≤100, zone_id?}`：区域跨摄像头 400、摄像头 404、
  模板 404；**同 behavior+zone+时间窗已存在 → 跳过**；返回 created/skipped/cameras/
  rules_per_camera；不自动启动；模板不含 camera_id（占位过 RuleCreate 校验后剥离）。

### 3. 前端（Zones.vue + api.js，路由零新增）

- 规则区「规则模板」块：模板下拉（含规则数）+ 套用目标下拉（全局/已有区域）+
  "套用到当前摄像头"（confirm）+ "把上方表单存为模板"；模板管理折叠区（列表/删除）；
  套用后刷新规则列表、失败显示 detail。

### 4. 测试（test_rule_templates.py，5 用例）

旧库迁移（表空/规则完好）、CRUD+校验矩阵（空 rules/非法 behavior/level=9/时间 8:0/
星期 9/非数组）、重名 409、PUT 冲突 409/404、套用 2 路（created=4、camera_id 归属、
112 字段随模板）、重复套用全跳过、区域校验（跨摄像头 400/正确挂 zone/404 链）、鉴权 401。
**过程坑（已修）**：api.py 缺 `import json` → POST 500（名字冲突竟未早暴露——补上）。

### 5. 实机验证（8188 临时实例）

存"实验室常规"（2 条）→ 套用 2 路 → `apply: {created:4, skipped:0}`、每路 2 条规则、
列表 rules_count=2 → **PASS**（套用 JSON 如上）。

### 6. 文档

35-api-reference（五端点+校验/跳重语义+与 80 并存说明）、部署FAQ「怎么复用一套规则」、
运维手册 §4.3（原 4.3 日历顺延 4.4）。

### 遗留

- 无。export/import 子端点规格标"可选"，未做（主链已闭环，需要时 0.5h 补）。
