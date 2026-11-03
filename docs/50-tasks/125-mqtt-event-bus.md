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
