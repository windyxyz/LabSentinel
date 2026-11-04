# 任务 125 · MQTT 事件总线（P2，生态开放第一步）

> 来源：flash 建议 #11（Frigate `comms/mqtt.py` + frigate-notify 消费模型）。
> 事件一旦发布就是对外契约：版本化、只加不减；默认关闭，broker 不可达不能影响主链路。
> 这是“未合并功能用外挂插件”的最低成本形态：HA/frigate-notify 式消费者可零改主服务接入。

## 目标

1. 新增 `server/app/mqtt_bridge.py`：后台单线程 + 有界队列，发布事件到 MQTT。
2. 依赖：`paho-mqtt`（纯 Python，加入 `server/requirements.txt`）；懒加载，未安装时状态=unavailable，
   不影响服务启动。
3. 配置（`LS_`，默认关）：`MQTT_ENABLED=false`、`MQTT_HOST=127.0.0.1`、`MQTT_PORT=1883`、
   `MQTT_USERNAME`、`MQTT_PASSWORD`、`MQTT_TLS=false`、`MQTT_TOPIC_PREFIX=labsentinel`、
   `MQTT_QOS=0`、`MQTT_RETAIN=false`、`MQTT_CLIENT_ID`（默认 `labsentinel-<hostname>-<rand>`）。
4. 发布时机：事件落库/派发时（含 folder 事件）；**不受告警静默/渠道过滤影响**（事件总线独立）。
5. 主题：`{prefix}/events/{camera_id}/{type}`；LWT/在线状态 `{prefix}/status`（online/offline）。
6. 负载（v1，字段只加不减）：
   ```json
   {"v":1,"device_type":"lab-monitor","id":123,"ts":"...","camera_id":1,
    "camera_name":"...","type":"enter_no_ppe","level":3,"track_id":0,
    "detail":"...","zones":["试剂区"],"has_snapshot":true,
    "snapshot_url":"/api/events/123/snapshot","api_base":"http://<lan>:8000"}
   ```
   `api_base` 由配置 `LS_PUBLIC_BASE_URL` 提供（默认空=不写）；**不含 token/本地绝对路径**。
7. 状态：`GET /api/mqtt/status`（enabled/connected/published/dropped/last_error，未配置也 200）；
   日志中密码不得出现；诊断包/MQTT_PASSWORD 加入脱敏清单。
8. 文档：`docs/73-mqtt-protocol.md`（主题、payload、订阅示例、版本策略、安全）；README/API 索引加链接。

## 实现建议

- `MqttBridge.start()` 在 lifespan 启动（enabled 才连；`connect_async`+`loop_start`，不阻塞启动）；
  `stop()` 先 flush（最多 2s）再 disconnect。
- 队列满（默认 1000）丢弃最旧/计数 `dropped`；发布失败重试由 paho 自动重连，不阻塞事件线程。
- `dispatcher.dispatch()` 在 WS 广播后调用 `mqtt.publish_event(event_row, camera_name, public_base)`；
  异常吞掉并计数。
- TLS：`MQTT_TLS=true` 用默认系统 CA；不提供跳过校验开关。
- 不引入认证/ACL 服务端；文档要求 broker 侧最小权限。

## 测试（`server/tests/test_mqtt.py`）

- 用 `sys.modules` 注入假 paho 客户端：enabled 时 connect/publish 主题与 payload 正确；
  disabled 时零连接/零发布；
- 队列满不阻塞、dropped 计数；broker 抛错不影响 dispatch/事件；
- payload 字段/版本/无密码/无绝对路径；`api_base` 配置生效；
- `/api/mqtt/status` 未配置 200；paho 未安装状态 unavailable；
- redact：`MQTT_PASSWORD` 不出现在诊断/日志测试样例。
- 真实 broker 可选：本机无 broker 就写 `⏳ BLOCKED: 无本地 MQTT broker`，不强求。

## 验收

- `pytest tests/ -q` 全绿；`requirements.txt` 更新；离线环境启动不报错；
- 报告附主题/payload 样例、队列/丢弃计数、broker 不可达行为。

## 约束

- 新依赖仅 `paho-mqtt`；不改告警渠道语义；不把 MQTT 当主告警通道（Webhook/厂商渠道仍在）；
- 不发布敏感截图路径/令牌；默认关闭。

## 执行报告

**flash 2026-11-04 完成（pytest 474 passed = 463+11 新增；paho-mqtt 已入 requirements 并装入测试 venv）**
（全量回归为 124 基线上 125 的口径，数字以本轮实跑为准）

### 1. server/app/mqtt_bridge.py（新文件）

- 懒加载 paho（未安装=unavailable，服务照常启动）；`connect_async+loop_start` 不阻塞启动；
  broker 连不上由 paho 重连，事件主链路零感知；
- 主题：`{prefix}/events/{camera_id}/{type}` + `{prefix}/status`（online retain + LWT offline）；
- **payload v1**（LocalSend 协议三要素：`v` 版本字段/`device_type: lab-monitor`/字段回显；
  **只加不减**）：含 id/ts/camera_id/camera_name/type/level/track_id/detail/zones([] 占位)/
  has_snapshot/snapshot_url/api_base（`LS_PUBLIC_BASE_URL`，空=空串）；**无令牌/无本地绝对路径**；
- 有界队列 1000（满丢弃+计数）；drain 线程发布失败只计数；`stop()` flush≤2s 后
  disconnect+发布 offline；
- 与告警链关系：`dispatcher.dispatch` 在 **WS 广播后**、静默判定**前**调用
  `publish_event`（try 吞错）——总线独立于静默/渠道过滤，语义与规格一致。

### 2. 接线

- config 10 项（LS_MQTT_* + LS_PUBLIC_BASE_URL，默认全关）；requirements.txt 加
  `paho-mqtt>=1.6,<2`（已装入测试 venv）；main.py lifespan start/stop；
- `GET /api/mqtt/status`（读保护；未配置也 200；`available:false`=paho 缺失降级说明）；
- 诊断包脱敏：redact.py SENSITIVE 已含 PASSWORD（测试断言 `MQTT_PASSWORD` 命中）。

### 3. 测试（test_mqtt.py，11 用例，假 paho/假 client 零网络）

disabled 零行为、topic+payload 断言、api_base 配置、start 连接参数+LWT+online、
paho 缺失降级、队列满 dropped、publish 抛错不外溢、drain 循环计数、status 两态、redact 覆盖。

### 4. 实机验证（8188，enabled=true + 无 broker 端口 31883）

服务照常启动；`/api/mqtt/status` → `{enabled:true, available:true, connected:false,
queue_depth:0}`；事件/接口全部照常 → **broker 不可达行为 PASS**。
⏳ **BLOCKED: 无本地 MQTT broker**（规格允许；真实 broker 联动由用户自建后按 docs/73 验证）。

### 5. 文档

新增 `docs/73-mqtt-protocol.md`（主题表/payload v1/HA 与 Python 订阅示例/版本策略/安全）；
README 文档索引 + 35-api-reference + .env.example 全部加上。

### 遗留

- 无。zones 字段 v1 恒空（预留，合并区域追踪后填充——契约只加不减已预留）。

### 主线复审补丁（2026-11-04）

- LWT `will_set` 原在 `connect_async` 之后调用，对本次连接可能不生效 → 移到连接前；
  在线状态 publish 保留在 `loop_start` 后（paho 排队）。
