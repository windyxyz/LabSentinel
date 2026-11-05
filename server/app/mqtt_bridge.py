"""MQTT 事件总线（任务125；Frigate comms/mqtt.py 蓝本 + frigate-notify 消费模型）。

定位：事件**只读外发**总线，不是告警通道（Webhook/厂商渠道照旧、静默/过滤不作用于它）。
契约：payload v1（`v`/`device_type` 字段，LocalSend 协议三要素），**只加不减**；
默认关闭（LS_MQTT_ENABLED=false）；paho-mqtt 懒加载（未安装=unavailable，不影响启动）；
broker 不可达/发布异常一律吞掉计数，绝不影响事件主链路；密码不进日志。
"""
import json
import logging
import os
import queue
import socket
import threading

from .config import settings

log = logging.getLogger("mqtt")

PAYLOAD_VERSION = 1
QUEUE_MAX = 1000

_q: "queue.Queue[dict]" = queue.Queue(maxsize=QUEUE_MAX)
_stats = {"published": 0, "dropped": 0, "failed": 0, "connected": False}
_client = None          # paho Client 实例（enabled 且可用时）
_lock = threading.Lock()
_unavailable_reason = ""


def _paho():
    """懒加载 paho-mqtt；未安装返回 None（状态=unavailable，服务照常启动）。"""
    global _unavailable_reason
    try:
        import paho.mqtt.client as mqtt
        return mqtt
    except ImportError as e:
        _unavailable_reason = f"paho-mqtt 未安装: {e}"
        return None


def _client_id() -> str:
    if settings.mqtt_client_id:
        return settings.mqtt_client_id
    return f"labsentinel-{socket.gethostname()}-{os.urandom(2).hex()}"


def build_payload(event: dict, camera_name: str | None) -> str:
    """payload v1（只加不减契约）。不含 token/本地绝对路径；zones v1 恒空列表（占位）。"""
    return json.dumps({
        "v": PAYLOAD_VERSION,
        "device_type": "lab-monitor",
        "id": event.get("id"),
        "ts": event.get("ts"),
        "camera_id": event.get("camera_id"),
        "camera_name": camera_name,
        "type": event.get("type"),
        "level": event.get("level"),
        "track_id": event.get("track_id"),
        "detail": event.get("detail"),
        "zones": [],
        # T-208（B-14）：派发前行里 snapshot 已 pop，has_snapshot 布尔由 _dispatch_row 写入；
        # 兼容直传完整行的调用方（或字段）
        "has_snapshot": bool(event.get("has_snapshot") or event.get("snapshot")),
        "snapshot_url": f"/api/events/{event.get('id')}/snapshot",
        "api_base": settings.public_base_url or "",
    }, ensure_ascii=False)


def topic_for(event: dict) -> str:
    return f"{settings.mqtt_topic_prefix}/events/{event.get('camera_id')}/{event.get('type')}"


def publish_event(event: dict, camera_name: str | None = None) -> bool:
    """事件落库/派发后调用（WS 广播之后）。入队即返回；异常吞掉计数，绝不抛。"""
    if not settings.mqtt_enabled:
        return False
    try:
        _q.put_nowait({"topic": topic_for(event),
                       "payload": build_payload(event, camera_name)})
        return True
    except queue.Full:
        _stats["dropped"] += 1  # 满丢最旧策略简化为丢最新+计数：主链路优先
        return False


def _drain_loop() -> None:
    while not _stop.is_set():
        try:
            item = _q.get(timeout=0.5)
        except queue.Empty:
            continue
        try:
            c = _client
            if c is None:
                _stats["dropped"] += 1
                continue
            info = c.publish(item["topic"], item["payload"],
                             qos=settings.mqtt_qos, retain=settings.mqtt_retain)
            if info.rc == 0:
                _stats["published"] += 1
            else:
                _stats["failed"] += 1
                log.warning("MQTT 发布失败 rc=%s", info.rc)
        except Exception as e:  # noqa: BLE001  broker 问题绝不外溢
            _stats["failed"] += 1
            log.warning("MQTT 发布异常（静默）: %r", e)


_stop = threading.Event()
_thread: threading.Thread | None = None


def start() -> bool:
    """lifespan 启动（enabled 才连）。返回是否启动。不阻塞：连接失败由 paho 重连。"""
    global _client, _thread
    if not settings.mqtt_enabled:
        return False
    mqtt = _paho()
    if mqtt is None:
        log.warning("MQTT 已启用但 paho-mqtt 不可用（%s）——事件总线降级关闭", _unavailable_reason)
        return False
    with _lock:
        if _client is not None:
            return True
        c = mqtt.Client(client_id=_client_id())
        if settings.mqtt_username:
            c.username_pw_set(settings.mqtt_username, settings.mqtt_password or None)
        if settings.mqtt_tls:
            c.tls_set()  # 系统默认 CA；不提供跳过校验开关
        # LWT 必须在 connect_async 之前设置（连接后再设对本次连接无效）
        c.will_set(f"{settings.mqtt_topic_prefix}/status", "offline",
                   qos=settings.mqtt_qos, retain=True)
        c.on_connect = lambda *a, **k: _stats.update(connected=True)
        c.on_disconnect = lambda *a, **k: _stats.update(connected=False)
        try:
            c.connect_async(settings.mqtt_host, settings.mqtt_port, keepalive=60)
            c.loop_start()
        except Exception as e:  # noqa: BLE001
            log.warning("MQTT 连接启动失败（服务照常）: %r", e)
            return False
        _client = c
        _stop.clear()
        _thread = threading.Thread(target=_drain_loop, name="mqtt-bridge", daemon=True)
        _thread.start()
        try:  # 在线状态（paho 排队，连接建立后发出）
            c.publish(f"{settings.mqtt_topic_prefix}/status", "online",
                      qos=settings.mqtt_qos, retain=True)
        except Exception:  # noqa: BLE001
            pass
        log.info("MQTT 事件总线已启动：%s:%s prefix=%s", settings.mqtt_host,
                 settings.mqtt_port, settings.mqtt_topic_prefix)
        return True


def stop() -> None:
    """flush（≤2s）后断开。"""
    global _client, _thread
    _stop.set()
    t, _thread = _thread, None
    if t is not None:
        t.join(timeout=2.0)
    c, _client = _client, None
    if c is not None:
        try:
            c.publish(f"{settings.mqtt_topic_prefix}/status", "offline",
                      qos=settings.mqtt_qos, retain=True)
            c.disconnect()
            c.loop_stop()
        except Exception:  # noqa: BLE001
            pass
        _stats.update(connected=False)


def status() -> dict:
    available = True
    if settings.mqtt_enabled and _client is None:
        mqtt = _paho()
        available = mqtt is not None
    return {"enabled": settings.mqtt_enabled, "available": available,
            "connected": _stats["connected"], "published": _stats["published"],
            "dropped": _stats["dropped"], "failed": _stats["failed"],
            "queue_depth": _q.qsize(),
            "host": settings.mqtt_host if settings.mqtt_enabled else "",
            "topic_prefix": settings.mqtt_topic_prefix if settings.mqtt_enabled else "",
            "error": _unavailable_reason if settings.mqtt_enabled and not available else ""}
