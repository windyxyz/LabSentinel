# 30 · 架构设计

> v0.1（2026-10-26）· 描述**当前已实现**的架构与关键决策。新增模块前先读本文，变更架构先写 ADR。

## 全局视图

```
┌─────────────────────────── 服务端 server/ (FastAPI + uvicorn) ───────────────────────────┐
│                                                                                          │
│  HTTP/WS API (api.py, main.py)                                                           │
│   ├── REST: cameras/zones/rules/events/stats/face(预留)                                  │
│   └── WS /ws/events ←── AlertDispatcher.hub                                              │
│                                                                                          │
│  CameraManager (pipeline.py)                                                             │
│   └── CameraWorker ×N（每路一个线程，daemon）                                             │
│        ├── 取流 (cv2.VideoCapture: rtsp/file/usb)  [按文件原速/实时流节流]                 │
│        ├── 分析调度（analysis_fps 限频，默认 5）                                          │
│        ├── Engine.detect_persons ──► ORT 会话（DML/VitisAI/CPU，_ACCEL_LOCK 串行）        │
│        ├── supervision.ByteTrack（跨帧 ID）                                              │
│        ├── zones.py 多边形判定（锚点=框底边中点）                                          │
│        ├── PpeEngine.detect_class（lab coat / no lab coat，证据分级）                      │
│        ├── RuleEngine 匹配（区域×时段×行为×级别×冷却）                                     │
│        ├── 事件落库 Store（SQLite）+ 截图快照 + 视频环形缓冲录制（异步写盘线程）             │
│        └── AlertDispatcher（WS 同步 + 6 外部渠道线程池异步）                               │
│                                                                                          │
│  Store (store.py): SQLite 单文件，字面量 SQL + 参数占位符                                   │
│                                                                                          │
└──────────────────────────────────────────────────────────────────────────────────────────┘
        ▲ HTTP/WS (127.0.0.1:8000, Vite 代理 /api /ws)
┌───────┴───────────┐         ┌──────────────────────────────┐
│ Web (Vue3+Vite)   │         │ Android (Flutter, M2 计划)    │
│ 仪表盘/摄像头/     │         │ 事件列表/详情/设置/WS 通知     │
│ 安全区/事件/报表    │         └──────────────────────────────┘
└───────────────────┘
```

## 分层与模块职责

| 层 | 模块 | 职责 | 不做什么 |
|---|---|---|---|
| 入口 | `main.py` | 应用装配、lifespan（DB/Manager/dispatcher 绑定）、WS 路由 | 不含业务逻辑 |
| API | `api.py` | REST 路由、参数校验、错误码 | 不直接碰推理 |
| 采集 | `pipeline.py` CameraWorker | 取流、节流、调推理、产事件 | 不做告警渠道细节 |
| 推理 | `engine.py` Engine/PpeEngine | ONNX 会话、letterbox、解码/NMS、**串行锁** | 不持有业务规则 |
| 跟踪 | supervision.ByteTrack | 跨帧关联 | — |
| 几何 | `zones.py` | 多边形包含、框重叠、ZoneState 状态机 | — |
| 规则 | `rules.py` | 规则匹配、冷却、时段窗口 | 不发通知 |
| 存储 | `store.py` | SQLite CRUD | 不做聚合（stats.py 独立） |
| 统计 | `stats.py` | 日/时聚合查询 | — |
| 告警 | `alerts.py` | WS Hub + 6 HTTP 渠道（白名单）+ SMTP | 不决定"要不要告警" |
| 配置 | `config.py` | pydantic-settings，LS_ 前缀，.env | 不硬编码路径 |

## 关键线程模型（重要）

- **主线程**：uvicorn/async 事件循环（API、WS 收发）
- **CameraWorker 线程 ×N**：阻塞式取流 + 同步推理。worker 内的任何阻塞（推理、写截图、SMTP）都会拖慢该路分析——因此：推理有全局锁（多路排队），SMTP 已改线程池，视频写盘有独立 recorder 线程。
- **WS 投递**：worker 线程 → `run_coroutine_threadsafe` → 事件循环（线程安全桥）
- **规则一致性**：Store 写操作带 `threading.Lock`；读用独立连接（SQLite WAL 未启用，读并发足够）

## 推理路径与后端策略

1. `settings.backend_order = directml,vitisai,cpu` 依次尝试；失败自动降级
2. VitisAI 后端自动选 XINT8 模型（`model_int8`），其余用 FP32（`model_fp32`）
3. **`_ACCEL_LOCK`（模块级）**：directml/vitisai 的 `sess.run` 全进程串行——DML 驱动并发缺陷曾致整机死机（80-risk-register A1）。CPU 后端不加锁
4. 人检与 PPE 是两个独立 ONNX 会话，均走 `Engine._run()`
5. 部署形态：CPU 档（无 GPU 机器）→ DML 档（本机 880M，64FPS）→ NPU 档（XINT8，40FPS，可选包）

## 事件模型（核心领域概念，术语见 docs/GLOSSARY.md）

```
进入安全区事件流：
  track 进入 zone → ZoneState.entered=True
    → PPE 判定（三级证据）：no(显式 No Lab Coat) / wearing(Lab Coat≥0.2) / unknown
    → 按行为类型匹配 RuleEngine（enter / enter_no_ppe / exit / occupancy_over）
    → Store.add_event(ts, camera, type, level, track, detail, snapshot, video)
    → AlertDispatcher.dispatch（WS 实时 + 外部渠道线程池）
```

**设计原则**：证据不足判 unknown 不判违规（防误报伤信任）；冷却期按 (rule, track) 维度；occupancy_over 用 track=0。

## 扩展点（预留给后续里程碑）

| 扩展点 | 位置 | 说明 |
|---|---|---|
| 新告警渠道 | `alerts.py` `_HttpPushChannel` 子类 + config + .env.example | 白名单/https/禁重定向三件套必须继承 |
| 新人脸模块 | `face.py`（任务05） | 默认关；CompreFace REST；同意登记表 |
| 新视频行为规则 | `pipeline._analyze` + `rules.py` | 例如滞留、跌倒（姿态模块）、抽烟 |
| 模型替换 | `LS_MODEL_FP32/INT8/PPE_MODEL` + `classes.txt` | 见 45-model-onboarding.md |
| 客户端 | Web/Flutter | 只消费 REST/WS，零 AI 依赖（ADR-0002） |

## 部署拓扑

- **一体机**（默认）：Win/macOS/Linux 单机，Docker Compose 或原生；数据卷 = `server/data/`（DB+截图+视频）
- **CS 分离**：分析机（GPU）+ 观察端（浏览器/手机），同一镜像
- 模型不入镜像：挂载 `models/`，安装脚本下载（协议与体积）

## 架构约束（写代码前必读）

1. 推理只走 `Engine._run()`；代码审查 grep `sess.run` 即拒
2. 新增 API 走 api.py 的 router + pydantic 模型；前端契约同步 docs/35-api-reference.md
3. SQL 一律字面量 + 参数占位符（Mimosa 静态扫描会拦拼接）
4. 外部 URL 必须域名白名单（防 SSRF）
5. 每路 worker 内不得有 >100ms 的阻塞操作（除推理排队）；慢操作入队/线程池
6. 前端不引入重型图表/UI 库（bundle 预算 gzip ≤100KB）
