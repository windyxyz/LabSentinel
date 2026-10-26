# 任务 12 · 事件闭环与误报回流（先读本目录 README.md）

> 规划依据：60-milestones M2C。目标：事件从"只读告警"变成"可处置工单"，并为数据飞轮积累误报样本。

## 背景

事件目前只有产生和查询，管理者无法标记"已处理/误报/忽略"。误报标注是数据飞轮（90-data-flywheel.md）的输入源，也是告警可信度的运营基础。

## 目标

### 12A 事件状态流（服务端）
- `events` 表加列：`status TEXT DEFAULT 'new'`（new | acked | false_positive | ignored）、`handled_by TEXT`、`handled_ts TEXT`
- 迁移函数兼容旧库（参考 04 视频任务的迁移写法）
- API（同步更新 docs/35-api-reference.md）：
  - `POST /api/events/{id}/status` body `{"status": "acked|false_positive|ignored", "note": "可选"}` → 200
  - `GET /api/events` 增加过滤参数 `status=`
  - Event 响应加 `status/handled_ts`
- 误报样本池：status=false_positive 时**复制**该事件截图到 `server/data/annotation_pool/`（不删原事件），文件名含事件 id 与时间——供任务 11 的预标注工具直接喂给 Roboflow

### 12B Web 端
- Events.vue：状态列 + 行内操作按钮（确认/误报/忽略，误报需确认弹窗）；状态筛选下拉
- 仪表盘：今日"未处理事件"计数卡片（/api/system 增加 `unhandled_counts` 或前端自行查询）

### 12C 统计
- stats 返回增加按状态过滤能力：`/api/stats/daily?status=`（可选参数）

## 验收标准

pytest 全绿 + 新增 ≥4 测试（状态迁移/非法 status 400/误报复制到池/旧库迁移）；`npm run build` 通过；API 文档同步。

## 执行报告

### 交付物
- **12A 服务端**：
  - `store.py`：events 表加 `status/handled_by/handled_ts` 三列（建表语句同步 + `_migrate` 旧库逐列补齐、幂等）；
    `EVENT_STATUSES` 常量；`events()` 重构为单条参数化 SQL（新增 status 过滤，'unhandled' 映射 new）；
    `set_event_status()`；`count_events_today_handled()`
  - `api.py`：`POST /api/events/{id}/status`（非法 status/回退 new → 400；误报时把截图**复制**到
    `server/data/annotation_pool/`，文件名 `ev{id}_{时间}_cam{cid}_{type}.jpg`，原截图保留）；
    `GET /api/events?status=`；`/api/system` 增加 `unhandled_today`
  - `config.py`：`annotation_pool_dir`（默认 data/annotation_pool，启动自动建目录）
  - `stats.py`：`daily()` 支持 status 过滤 → `/api/stats/daily?status=`
- **12B Web**：Events.vue 加状态列（色标：new 红/acked 黄绿/其他橙）、行内 确认/误报/忽略 按钮
  （误报有 confirm 弹窗；已处置显示处理时间）、状态下拉筛选；Dashboard 加"未处置"计数卡片（橙色高亮>0）
- **12C 统计**：如上 stats 过滤

### 验收结果
- `pytest tests/ -q`：**46 passed**（新增 5 个：旧库迁移+幂等/状态流转/非法与回退 400/状态过滤+unhandled 映射/
  API 全链路含误报复制到池与原件保留）
- `npm run build`：通过（gzip 37.5KB）
- 手动链路推演：误报标记 → pool 文件出现 → `preannotate.py --images data/annotation_pool` 直接可用（飞轮闭环接通）

### 遗留问题
- 处置人固定 "admin"（无鉴权体系，body 的 handled_by 可传）；接登录体系时替换
- 误报池目前只收截图；视频误报样本（task 04 产物）未入池——等飞轮需要时再扩
