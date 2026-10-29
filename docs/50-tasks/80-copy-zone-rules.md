# 任务 80 · 区域/规则批量复制到多路摄像头（P1）

> 背景：多路摄像头通常安全区布局相同、规则相同；现在只能逐路重画/重建。本任务提供“从一路复制到
> 多路”的 API 与界面，复制时正确映射 zone_id，避免规则指向源摄像头的区域。

## 目标

### 1. API `POST /api/cameras/{cid}/copy-config`（require_admin）

body：
```json
{"target_ids":[2,3,4],"copy_zones":true,"copy_rules":true,"name_suffix":"-copy"}
```

- 源摄像头必须存在；target_ids 非空、去重、排除源自身；任一目标不存在 → 404（**先全量校验再写**）。
- copy_zones：复制源摄像头的所有区域到每个目标；区域名冲突（同目标已有同名）→ 跳过并计数；
  可加 `name_suffix`（默认空）。
- copy_rules：复制源摄像头所有规则；`zone_id` 映射到目标摄像头对应区域（按 **区域名** 匹配）；
  源规则 zone 为空（全画幅）→ 目标 zone_id 也为空；目标已有同 behavior+zone+时间段规则 → 跳过。
- 返回：
  ```json
  {"zones_created":n,"zones_skipped":n,"rules_created":n,"rules_skipped":n,
   "targets":[{"camera_id":2,"zone_map":{"源zone名":"新zone_id"}}]}
  ```
- 不启动摄像头、不改源配置；不复制事件。

### 2. 前端（`web/src/views/Zones.vue` 或 `Cameras.vue`）

- 在安全区配置页增加「复制到其他摄像头」区块：
  - 源摄像头 = 当前选中；目标多选（checkbox 列表，排除源）；
  - 勾选“复制区域/复制规则”；
  - 点击后 confirm，显示 created/skipped 摘要；失败显示 detail；
  - 目标列表为空时提示先添加摄像头。
- 只改一个页面（优先 Zones.vue，因为区域在那里配置），不新增路由，不改全局四件套。

### 3. 测试 `server/tests/test_copy_config.py`

- 源 1 区域 + 2 规则（含 zone 与全画幅各一）→ 复制到 2 个目标：
  - 每目标 1 区域、2 规则；规则 zone_id 指向**目标**区域而非源区域；
  - 全画幅规则 zone_id 为 None；
- 目标已有同名区域/同规则 → skipped 计数正确、不重复插入；
- target_ids 含不存在/源自身/空 → 400/404 且**零写入**；
- copy_zones=false 但 copy_rules=true 且目标无同名区域 → 规则跳过并 warning（不要指向错误 zone）；
- 鉴权 401。

### 4. 文档

- `docs/35-api-reference.md` 新增端点；
- `docs/部署FAQ.md`「多路摄像头怎么快速套用同一套区域/规则」；
- `docs/运维手册.md` 多路部署一节加命令。

## 约束

- 先校验后写，失败不留半截；不复制事件/截图；不自动启动；
- 区域名映射必须精确（大小写敏感）；不引入依赖；
- 前端只改一个 view，不新增路由。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 临时库实测：1→2 复制后查询目标规则 zone 指向正确；重复复制 skipped；
- 报告附复制前后 JSON 对照。

## 执行报告

（待填：改动文件 / 复制映射证据 / 测试与 build / 遗留）
