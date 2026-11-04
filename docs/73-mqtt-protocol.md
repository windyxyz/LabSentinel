# MQTT 事件总线协议（任务125，v1）

> LabSentinel 把事件**只读外发**到 MQTT，供 Home Assistant / frigate-notify 式消费者
> 零改主服务接入。这不是告警通道：Webhook/企业微信等渠道照旧，**告警静默与渠道过滤
> 不作用于事件总线**（总线永远发全部事件）。

## 1. 开启

`.env`（或便携 config.env）：

```
LS_MQTT_ENABLED=true
LS_MQTT_HOST=127.0.0.1        # broker 地址
LS_MQTT_PORT=1883
LS_MQTT_USERNAME=             # 可选
LS_MQTT_PASSWORD=             # 可选（自动进诊断包脱敏）
LS_MQTT_TLS=false             # true=系统默认 CA；无跳过校验开关
LS_MQTT_TOPIC_PREFIX=labsentinel
LS_MQTT_QOS=0
LS_MQTT_RETAIN=false
LS_PUBLIC_BASE_URL=           # payload api_base 用；空=不写
```

依赖 `paho-mqtt`（`server/requirements.txt` 已含；懒加载——未安装时服务照常启动，
`/api/mqtt/status` 显示 `available:false`）。状态端点：
`GET /api/mqtt/status` → `{enabled,available,connected,published,dropped,failed,queue_depth,…}`。

## 2. 主题

| 主题 | 方向 | 说明 |
|---|---|---|
| `{prefix}/events/{camera_id}/{type}` | 发布 | 每条事件一条（含 folder 源违规事件） |
| `{prefix}/status` | 发布+LWT | `online`/`offline`（retain；异常掉线由 broker LWT 代发） |

## 3. 事件 payload（v1，字段只加不减）

```json
{"v":1,"device_type":"lab-monitor","id":123,"ts":"2026-11-04T09:00:00",
 "camera_id":1,"camera_name":"化学一","type":"enter_no_ppe","level":3,
 "track_id":0,"detail":"…","zones":[],"has_snapshot":true,
 "snapshot_url":"/api/events/123/snapshot","api_base":"http://<lan>:8000"}
```

- `v`：payload 版本，不兼容变更才升；**字段只加不减**，消费端按 `v` 分流；
- `zones`：v1 恒为 `[]`（预留；区域追踪合并后填充）；
- `api_base`：来自 `LS_PUBLIC_BASE_URL`（空=空串），便于消费端拼完整截图 URL；
- **不含**令牌/本地绝对路径/人脸数据；截图不外发，消费端按 `snapshot_url` 自取（读保护
  开启时需自配令牌）。

## 4. 订阅示例（Python，frigate-notify 式消费者）

```python
import paho.mqtt.client as mqtt
def on_message(c, u, m):
    ev = json.loads(m.payload)
    if ev["type"] == "enter_no_ppe" and ev["level"] >= 3:
        ...
c = mqtt.Client()
c.on_message = on_message
c.connect("127.0.0.1", 1883)
c.subscribe("labsentinel/events/#")
```

Home Assistant：用 MQTT 集成订阅 `labsentinel/events/#`，按 `type`/`level` 建自动化。

## 5. 安全与边界

- broker 建议：内网部署 + 最小权限账号（只写 `labsentinel/#`）；密码进 `.env`
  **不进仓库/诊断包明文**（诊断包自动脱敏）；
- 队列上限 1000，满则丢弃+计数（`/api/mqtt/status` 可见）——主链路优先；
- broker 不可达：paho 自动重连，事件照常入库/告警；服务启停不受影响；
- **不自动下载/安装 broker**；本机无 broker 时状态 `connected:false` 属预期。
