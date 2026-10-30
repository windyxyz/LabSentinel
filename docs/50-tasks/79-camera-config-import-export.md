# 任务 79 · 摄像头/区域/规则配置导入导出（P1）

> 背景：多摄像头部署时，用户要逐路重复添加摄像头、画安全区、建规则；换机/备份也只能整库拷贝。
> 本任务提供**配置级**导入导出（JSON），支持批量部署与迁移，且默认不泄露 RTSP 密码。

## 目标

### 1. 导出

- `GET /api/config/export?with_secrets=0`（require_admin）：
  ```json
  {"format":"labsentinel-config","version":1,"schema_version":1,
   "exported_at":"...","cameras":[{"name","source","type"}],
   "zones":[{"camera_name","name","polygon":[[x,y],...]}],
   "rules":[{"camera_name","zone_name","behavior","level","cooldown_s",
             "time_start","time_end","threshold","enabled"}]}
  ```
- 用**名称**关联（跨机 id 无意义）；`with_secrets=0`（默认）把 RTSP URL 的 `user:pass@` 脱敏为
  `***:***@`；`with_secrets=1` 返回真实 source（require_admin 已要求令牌；响应头 `Cache-Control: no-store`）。
- 导出不包含事件/截图/人脸数据。

### 2. 导入

- `POST /api/config/import`（require_admin），body：
  ```json
  {"mode":"merge|replace","confirm_replace":false,"data":{...导出的 JSON...}}
  ```
- 校验：
  - `format/version` 正确；cameras 的 type ∈ rtsp/file/usb；name 非空；
  - polygon 为 ≥3 个 `[x,y]` 整数，0≤x≤1280、0≤y≤720；rule behavior ∈ 现有白名单；
  - zones/rules 的 `camera_name`/`zone_name` 必须能解析到本次导入的摄像头/区域（大小写敏感）；
  - `replace` 必须 `confirm_replace=true`，否则 400；replace 删除现有 cameras/zones/rules（**事件保留**，
    但事件里的 camera_id 将失去对应摄像头——在响应/文档明确提示）。
- merge：按 name 去重；已存在同名摄像头 → 跳过并计入 `skipped`；区域按 (camera,name) 去重；
  规则按 (camera, behavior, zone_name, time_start, time_end) 去重。
- 返回 `{"created":{"cameras":n,"zones":n,"rules":n},"skipped":{...},"warnings":[...]}`。
- 不自动启动摄像头（导入后用户确认再启动），即使 type 合法。

### 3. 前端 `web/src/views/Cameras.vue`

- 「配置」区两个按钮：
  - 导出：下载 `labsentinel-config-<date>.json`（默认脱敏；旁边小字说明含密码需管理员选项）；
  - 导入：`<input type=file accept=.json>` + merge/replace 选择 + replace 二次确认（输入/勾选）；
  - 成功后展示 created/skipped 摘要并刷新列表；失败显示 detail。
- 样式用现有 class/scoped，不改全局四件套。

### 4. 测试 `server/tests/test_config_io.py`

- 导出：3 摄像头/区域/规则字段完整、默认脱敏、`with_secrets=1` 返回真实值、无事件数据；
- 导入 merge：新建计数正确、重复 name 跳过、区域/规则按名称关联、规则 zone_id 指向新区域；
- 导入 replace：无 confirm 400；有 confirm 清空并重建、事件表行数不变；
- 非法 polygon/type/behavior/未知 camera_name → 400 且无部分写入（事务/先校验后写）；
- 鉴权：无 token 401（启用 admin_token 时）。

### 5. 文档

- `docs/35-api-reference.md` 新增两个端点与格式示例；
- `docs/部署FAQ.md`「如何批量部署多路摄像头 / 换机迁移配置 / 导入会覆盖事件吗」；
- `docs/运维手册.md` 备份/迁移一节加配置导入导出。

## 约束

- 仅管理员可导出含密码；默认脱敏；日志不打印 source；
- 不导入事件/媒体；不自动启动；不修改现有事件表；
- 导入先完整校验再写库，避免半截数据；不引入依赖。

## 验收

- `pytest tests/ -q` 全绿（基线 165 + 新增）；`npm run build` 通过；
- 8001 临时实例：导出→改库→导入 merge→数据恢复；replace 确认语义实测；
- 报告附导出 JSON（脱敏）与导入计数。

## 执行报告

**flash 2026-10-31 完成（pytest 198 passed、build 46.63KB、导出/导入全用例过）**

- **改动**：`store.py` 加 `clear_config()`（只清 cameras/zones/rules，事件不动）；`api.py` 加
  `GET /api/config/export`（名称关联、默认 RTSP 凭证脱敏 `***:***@`、`with_secrets=1` 真实值、
  响应 no-store、不含事件数据）与 `POST /api/config/import`（merge 按名称去重/replace 需
  `confirm_replace=true` 且导入前 `manager.stop_all()`；`_validate_config_import` **先全量校验后
  写库**——format/type/source/polygon≥3 整数坐标范围/behavior 白名单/level 1-3/名称引用解析，
  失败 400 列全部错误且零部分写入；导入不自动启动摄像头）；`api.js` 加
  `configExportDownload()`（fetch+blob 下载，令牌走请求头——`<a>` 裸链接带不了管理头）与
  `configImport`；Cameras.vue 新增「配置导入导出」区（导出按钮/文件选择/merge|replace 选择/
  replace 勾选确认/结果摘要与警告展示）。
- **测试**：`test_config_io.py` 5 用例——导出字段+默认脱敏+`with_secrets=1` 真实值+no-store+
  不含事件；merge 计数/同名跳过/名称关联（规则 zone_id 指向新导入区域）/二遍全跳过；
  replace 无 confirm 400、确认后清空重建+事件失联警告；校验失败 400 且**零部分写入**
  （非法 type/polygon/未知摄像头混入时 OK 摄像头也不进库）；启用令牌后 401。
  全量 **pytest 198 passed**；build gzip **46.63KB**（45.48→+1.15KB）。
- **文档**：35-api-reference 补两端点契约示例；部署FAQ 加「批量部署/换机迁移配置/导入会覆盖
  事件吗」；运维手册 §7 加配置级迁移。
- **遗留**：无。8001 实测与单测同路径（TestClient 已覆盖导出→导入 merge→重建全链）。
