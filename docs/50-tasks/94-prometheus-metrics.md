# 任务 94 · Prometheus 指标端点（P2）

> 背景：如果要长期挂机，运维希望接监控（Grafana/Prometheus）看摄像头 fps、事件量、磁盘、投递失败。
> 本任务提供只读 `/api/metrics` 文本指标，不引入客户端库。

## 目标

### 1. `GET /api/metrics`（读保护口径；`text/plain; version=0.0.4`）

输出示例：
```
# HELP labsentinel_up 1=alive
# TYPE labsentinel_up gauge
labsentinel_up 1
labsentinel_uptime_seconds 123
labsentinel_cameras_total 4
labsentinel_cameras_running 3
labsentinel_camera_fps{camera_id="1"} 25.0
labsentinel_camera_frame_age_seconds{camera_id="1"} 0.2
labsentinel_events_today{level="3"} 2
labsentinel_alert_deliveries{status="failed"} 0
labsentinel_disk_free_bytes 1.23e+11
labsentinel_db_size_bytes 409600
labsentinel_schema_version 4
```
- 只输出数值/有限标签；**不要**输出摄像头名称/RTSP 地址（避免泄露）。
- 摄像头指标来自 `manager.status()`；事件今日来自 `store.today_event_counts()`；投递来自
  `dispatcher.store.delivery_counts()`；磁盘/DB 用 `shutil`/`stat`；异常降级为不输出该指标（不 500）。
- 不做鉴权以外的限流；不支持 pushgateway。

### 2. 测试 `server/tests/test_metrics.py`

- TestClient 200、content-type 含 `text/plain`；包含 up/cameras/events/disk 行；
- 无摄像头时 cameras_total=0、无 camera_fps 行；
- 标签只含 camera_id/level/status；不出现测试摄像头名称/source；
- 读保护开时无 token 401；
- 数值格式可被正则解析。

### 3. 文档

- `docs/运维手册.md` 加「接入 Prometheus/Grafana」示例（scrape_config 片段）；
- `docs/35-api-reference.md` 端点；
- README 功能一览可选一行。

## 约束

- 不引入 prometheus_client；不暴露敏感标签；不改变现有端点；
- 读保护口径与其它 GET 一致；
- 指标生成要快（<50ms，避免每次遍历大文件；DB size 用 stat）。

## 验收

- `pytest tests/ -q` 全绿；
- 8001 实跑 `curl /api/metrics`（或 httpx）输出可解析；抓取耗时记录；
- 报告附指标样例（无敏感值）。

## 执行报告

（待填：改动文件 / 指标样例 / 测试 / 遗留）
