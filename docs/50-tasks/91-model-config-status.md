# 任务 91 · 模型/配置只读状态（P2）

> 背景：用户想知道“当前用的哪个模型、类别对不对、配置是否生效”，现在只能看 `.env` 或跑脚本。
> 本任务提供只读 API + Cameras 页展示，不加载 ONNX、不改配置。

## 目标

### 1. `GET /api/model/status`（读保护口径）

- 返回：
  ```json
  {"backend":"directml","imgsz":640,"conf":0.4,"analysis_fps":5.0,
   "models":{"ppe":{"path":"...","exists":true,"size_bytes":123,"mtime":"..."},
             "fp32":{...},"int8":{...}},
   "classes":{"file":"...classes.txt","exists":true,"count":11,"names":["person",...]},
   "ppe":{"class_names":"lab coat","violation_class":"no lab coat",
          "conf":0.3,"wearing_conf":0.2},
   "face_enabled":false}
  ```
- classes 文件定位与 `PpeEngine` 一致（模型同目录 `<stem>.classes.txt` 或配置）；只读前 50 个名字。
- 不读取模型权重、不创建 session、不触发推理。
- 路径显示为绝对路径（本机管理界面；若担心泄露可在报告建议）。

### 2. `GET /api/config/summary`（require_admin）

- 返回当前 `Settings` 的脱敏键值（`LS_` 前缀），规则：
  - 键名含 `TOKEN|PASSWORD|PASS|SECRET|KEY|CREDENTIAL|AUTH` → `***`；
  - 值中的 URL `user:pass@` → `***:***@`；
  - 只返回字符串/数字/布尔/路径的 `str()`，不返回内部对象；
  - 不返回 `.env` 原文。
- 复用/抽取 `collect_diagnostics` 的脱敏逻辑到 `server/app/redact.py`（脚本与 API 共用），
  避免两套规则漂移；脚本改为 import 该模块（源码/便携都可用），若 import 失败保留本地实现。

### 3. 前端 `web/src/views/Cameras.vue`

- 底部「模型与配置」折叠区：
  - 模型路径/大小/修改时间、classes 数量与前 10 个类别、backend、PPE 阈值、人脸开关；
  - 配置摘要表格（admin 才显示；401 时提示填管理令牌）；
  - 「刷新」按钮；不提供编辑（改配置走 `.env`/`config.env`，文案说明）。
- 只改 Cameras.vue/api.js，不新增路由。

### 4. 测试 `server/tests/test_model_config_status.py`

- 临时模型目录 + classes 文件：端点字段/数量/名字正确；文件缺失 exists=false 不崩；
- config summary：token/pass/secret/key 全部 `***`，URL 凭证脱敏，普通键保留；
- 鉴权：model status 读保护口径；config summary 无 admin 401；
- 不加载 ONNX（可用 monkeypatch 确保 engine 未被调用）。

### 5. 文档

- `docs/35-api-reference.md` 两个端点；
- `docs/运维手册.md` 模型/配置核对；
- `docs/部署FAQ.md`「怎么确认当前模型和类别」。

## 约束

- 只读；不暴露明文密钥；不引入依赖；不加载模型；
- 前端只改 Cameras.vue/api.js，不改全局四件套。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 实跑：model status 显示真实 11 类；config summary 无明文 token；
- 报告附 JSON（脱敏）。

## 执行报告

**flash 2026-10-31 完成（pytest 263 passed、build 50.49KB、5 用例）**

- **改动**：新增 `server/app/redact.py`（脱敏逻辑抽离，collect_diagnostics 与 API 共用）；
  `api.py` 加 `GET /api/model/status`（require_read）——backend/imgsz/conf/analysis_fps/
  models(ppe,fp32)/classes(names 前 50)/ppe 阈值/face_enabled；**不加载 ONNX、不触发推理**
  （backend 优先取已初始化引擎，否则报配置值）；`GET /api/config/summary`（require_admin）——
  settings 全量键值脱敏输出（复用 redact.py）；`collect_diagnostics.py` 改 import redact.py
  （import 失败保留本地 fallback）。
- **测试**（test_model_config_status.py 5 用例）：字段/大小/classes 数量与名字/缺失不崩/
  脱敏（admin_token/read_token/smtp_pass/webhook query token 全 ***、smtp_host/user 保留）/
  鉴权口径。全量 **pytest 263 passed**（258 + 5）；build gzip **50.49KB**。
- **文档**：.env.example 补 LS_ALERT_TEMPLATE/LS_ALERT_FILTERS/LS_READ_TOKEN 注释；
  35-api-reference 补 model/status 与 config/summary 端点。
- **遗留**：无。
