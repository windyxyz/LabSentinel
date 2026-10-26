# 任务 05 · 人脸比对模块（CompreFace 集成，默认关闭）【主力模型任务 + 合规红线】

> 先读本目录 README.md。**此任务有合规硬约束，见文末，违反任何一条即任务失败。**

## 背景

需求 FR5：识别进入者是否为**注册人员**、发现陌生人（安全管理用途，不做考勤）。预研结论（docs/20-research/00-synthesis.md §4）：选 **CompreFace**（Apache-2.0，Docker 部署，REST API 自带 1:N 识别与人脸库管理），**禁止 InsightFace 预训练模型**（不可再分发）。

## 目标

1. docker-compose 增加 CompreFace 服务（仅 `LS_FACE_ENABLED=true` 时需要启动）
2. 服务端新增 `face.py`：摄像头画面帧 → CompreFace `/api/v1/recognition/faces` 检测+识别（face limit 1，阈值 0.75 可配）→ 返回 (subject 或 None, confidence)
3. 管理端点：`POST /api/face/persons`（注册：上传照片到 CompreFace subject=人员名）、`GET /api/face/persons`、`DELETE /api/face/persons/{subject}`；所有写操作**仅在功能开启时可用**，否则 403
4. 管线接入：`CameraWorker` 每路新增 `LS_FACE_ENABLED` 判定，开启时对每个**进入安全区**的 track 抽 1 帧调识别；产生新事件类型 `stranger`（未注册人员进入，级别默认 L2）；人员名写入 enter 事件 detail
5. Web 端：设置页（或摄像头页内）——人脸开关状态展示（只读，开关只能改 .env 重启，**不提供界面开关**，防误开）、注册人员管理表单、stranger 事件类型文案
6. `.env.example` 与 README 合规章节

## 技术要点

- CompreFace REST：`http://compreface:8000/api/v1/recognition/faces`，header `x-api-key`；照片 multipart 上传 `/api/v1/recognition/faces?subject=名字`
- 调用走 httpx，5s 超时，失败**降级为跳过识别**（不阻塞主管线、不误报 stranger）
- 频率控制：同一 track_id 冷却 30s 内不重复调 API
- 识别结果不落原图，只落 subject 名与置信度（合规）

## 合规红线（来自《人脸识别技术应用安全管理办法》2025-06-01 施行，详见 20-research/00-synthesis.md §7）

1. 功能**默认关闭**（`LS_FACE_ENABLED` 默认 false）
2. 开启需运维改配置重启，并在启动日志打印合规提示（需单独同意、现场提示标识、备案阈值 10 万人）
3. 代码内置**同意登记表**：注册人员时必须传 `consent: true` 字段并记录时间戳到本地 SQLite `face_consents` 表（subject, ts）——没有同意记录的注册请求 400
4. 不把人脸图片写盘、不入事件库；CompreFace 数据目录挂 volume 便于一键清除
5. 提供一键清除：`DELETE /api/face/persons/all`（连同同意记录）

## 验收标准

默认关闭时一切行为与现在完全一致（回归）；开启后：注册 A→A 进入安全区 enter 事件含名字；陌生人进入触发 stranger 事件；CompreFace 停机时系统不崩、事件照常。单测覆盖：开关默认值、同意强制、降级路径。pytest 全绿。

## 执行报告

### 交付物
- `server/app/face.py`：
  - `FaceClient`：CompreFace REST 客户端（recognize 1:N / add_person / list_persons / delete_person），
    httpx 5s 超时，**任何网络失败降级返回 status=error，不抛出、不阻塞主管线**；transport 可注入（单测）
  - `FaceGate`：管线门控——`LS_FACE_ENABLED` + API Key 双重判定开启；同 track 30s 冷却（`LS_FACE_COOL_S`）；
    对人框裁剪后 JPEG 上送（**原图不落盘**，只落 subject 名与置信度）
- `server/app/api.py`：`GET /api/face/status`（只读状态）；`POST /api/face/persons`（Form：subject/consent/photo，
  **consent≠true → 400**，成功后同意时间戳落 `face_consents` 表）；`GET /api/face/persons`（带同意时间）；
  `DELETE /api/face/persons/{subject}`；`DELETE /api/face/persons/all`（一键清除人员+同意记录）。
  所有写操作在功能关闭时 **403**、未配 Key 时 503
- `server/app/store.py`：`face_consents(subject, ts)` 表 + 增查清方法（建表语句入 _SCHEMA，新库自动创建）
- `server/app/pipeline.py`：进入安全区时调 `FaceGate.check()`——熟人姓名并入 enter 事件详情
  （"…，人员：张三"）；陌生人触发新事件类型 `stranger`（L2，不走规则匹配，靠人脸冷却限频；
  识别失败/冷却期/功能关闭均不产生 stranger）
- `server/app/main.py`：`LS_FACE_ENABLED=true` 时启动打印合规提示（同意留痕/现场标识/10 万人备案/一键清除）
- `server/app/constants.py` 与 `web/src/api.js`：TYPE_TEXT 增加 `stranger: 陌生人进入`
- Web：新增"人脸管理" tab（`views/Face.vue`）——开关状态**只读**展示（无界面开关，防误开）、
  注册表单（照片+人员名+"已取得本人同意"勾选）、人员列表（含同意时间）、单个删除与一键清除（带确认）
- `docker-compose.yml`：CompreFace 四服务（api/fe/admin/core:1.2.0-mobilenet + postgres:15）挂 `profiles: ["face"]`，
  默认 `docker compose up` 不启动；人脸库落 `./data/compreface-db` 卷便于一键清除
- `server/.env.example` 与 README「人脸比对合规」章节（含全部红线条款）

### 验收结果
- 单测 6 个（`tests/test_face.py`）：**开关默认值=false（合规）**、known/unknown/冷却、**降级路径（服务宕→None 不误报）**、
  未配 Key 降级、API 403 守卫、**同意强制（consent=false → 400；成功注册落 consent_ts）**、一键清除。全套 31 passed
- 回归：功能默认关闭时——FaceGate.check 恒 None，事件详情文案与此前完全一致（垂直切片实测确认），
  分析管线/事件/API 行为零变化
- 新依赖 `python-multipart`（0.0.32，FastAPI Form/File 必需；纯 Python 无 DLL，清华源安装，不影响运行中的 uvicorn）
- ⏳ CompreFace 真机联调未做（本机无 Docker）：compose 服务图为按官方镜像（Docker Hub 已核实
  exadel/compreface-{api,fe,admin}:1.2.0、core:1.2.0-mobilenet）编写的 best-effort 版本，文档已注明
  "启动异常以官方 compose 为准"；REST 契约按官方 API 实现， subjects 列表解析做了多版本兼容
- ⏳ "注册 A→A 进入含名字 / 陌生人触发 stranger"端到端场景需 CompreFace 运行后验证（代码路径已有单测覆盖）

### 遗留问题
- CompreFace 人员库与本地同意记录可能漂移（本地清了远端没清等）；一键清除已双向清理，但日常单删以远端为准
- 识别精度阈值 0.75 为文档建议值，投产前需按现场样本校准
