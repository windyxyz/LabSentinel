"""本地 VLM 误报预分类（任务124；Frigate genai 蓝本 + Immich 微服务形态）。

铁律：
- 默认关闭（LS_GENAI_PROVIDER=none）→ 零网络请求、零行为变化；
- 只写 events.ai_review 标记（"仅供参考"），**绝不改 status / 不抑制告警**；
- 失败静默（WARNING+计数）：VLM 挂了不影响事件、投递、WS；
- 截图只发回环地址；非回环必须显式 LS_GENAI_ALLOW_REMOTE=true（防误配外发截图）；
- 独立单线程队列（maxsize=100，满即丢弃+计数），不阻塞 pipeline/API；
  VLM 走独立服务进程（Ollama 等），不复用 Engine._run()；同机 GPU 可能竞争——
  默认并发 1，24h/压测场景建议关闭（文档已写）。
"""
import base64
import json
import logging
import queue
import threading
from urllib.parse import urlparse

import cv2
import httpx
from pydantic import BaseModel, ConfigDict, ValidationError

from .config import settings
from .vision import read_image

log = logging.getLogger("genai")

VERDICTS = ("likely_true", "likely_false", "uncertain")
QUEUE_MAX = 100
IMAGE_LONG_EDGE = 1280

PROMPT = (
    "你是实验室安全监控复核员。看这张事件截图，判断：画面中是否确有人员未穿实验服/进入禁区？"
    '只输出 JSON：{"verdict":"likely_true|likely_false|uncertain",'
    '"reason":"<=200字","confidence":0-1}。'
    "无法判断时 verdict=uncertain，不要臆测。"
)


class AIReview(BaseModel):
    model_config = ConfigDict(extra="forbid")  # 严格 schema（Immich 同款）
    verdict: str
    reason: str = ""
    confidence: float = 0.0


class GenAIError(Exception):
    pass


def _assert_loopback(url: str) -> None:
    host = (urlparse(url).hostname or "").lower()
    if host in ("127.0.0.1", "localhost", "::1", "[::1]"):
        return
    if not settings.genai_allow_remote:
        raise GenAIError(f"非回环地址 {host} 被拒绝（防误配外发截图；"
                         "确需远程 VLM 设 LS_GENAI_ALLOW_REMOTE=true）")


def _compress_jpeg(path: str) -> bytes | None:
    """读截图并压缩到长边 ≤1280 的 JPEG；失败返回 None（跳过复核）。

    必须走 vision.read_image（np.fromfile+cv2.imdecode）：Windows 上 cv2.imread 对
    含中文的安装路径（本项目默认 D:\\...\\监控识别）会静默返回 None。
    """
    img = read_image(path)
    if img is None:
        return None
    h, w = img.shape[:2]
    scale = max(w, h) / IMAGE_LONG_EDGE
    if scale > 1:
        img = cv2.resize(img, (int(w / scale), int(h / scale)))
    ok, buf = cv2.imencode(".jpg", img, [cv2.IMWRITE_JPEG_QUALITY, 80])
    return buf.tobytes() if ok else None


def _strip_fences(text: str) -> str:
    t = (text or "").strip()
    if t.startswith("```"):
        t = t.strip("`")
        if t.startswith("json"):
            t = t[4:]
    return t.strip()


def _call_provider(b64_image: str) -> str:
    """调 provider 返回原始文本。HTTP 错误/超时抛 GenAIError。"""
    provider = settings.genai_provider
    timeout = settings.genai_timeout_s
    try:
        if provider == "ollama":
            r = httpx.post(
                f"{settings.genai_url.rstrip('/')}/api/generate",
                json={"model": settings.genai_model, "prompt": PROMPT,
                      "images": [b64_image], "stream": False, "format": "json",
                      "options": {"temperature": 0}},
                timeout=timeout)
        elif provider == "openai_compat":
            headers = {}
            if settings.genai_api_key:
                headers["Authorization"] = f"Bearer {settings.genai_api_key}"
            r = httpx.post(
                f"{settings.genai_url.rstrip('/')}/v1/chat/completions",
                json={"model": settings.genai_model, "temperature": 0, "max_tokens": 300,
                      "messages": [{"role": "user", "content": [
                          {"type": "text", "text": PROMPT},
                          {"type": "image_url",
                           "image_url": {"url": f"data:image/jpeg;base64,{b64_image}"}}]}]},
                headers=headers, timeout=timeout)
        else:
            raise GenAIError(f"未知 provider: {provider}")
    except httpx.HTTPError as e:
        raise GenAIError(f"VLM 请求失败：{type(e).__name__}") from e
    if r.status_code != 200:
        raise GenAIError(f"VLM 返回 HTTP {r.status_code}")
    data = r.json()
    if provider == "ollama":
        return str(data.get("response") or "")
    try:
        return str(data["choices"][0]["message"]["content"] or "")
    except (KeyError, IndexError, TypeError) as e:
        raise GenAIError("VLM 响应缺 choices/message") from e


def parse_review(text: str) -> AIReview:
    """剥离 markdown 围栏 → JSON → pydantic 严格校验 + verdict 白名单。"""
    try:
        raw = json.loads(_strip_fences(text))
    except ValueError as e:
        raise GenAIError("VLM 输出不是 JSON") from e
    try:
        review = AIReview.model_validate(raw)
    except ValidationError as e:
        raise GenAIError(f"VLM 输出字段非法：{e.errors()[0].get('type')}") from e
    if review.verdict not in VERDICTS:
        raise GenAIError(f"verdict 非法: {review.verdict}")
    review.confidence = max(0.0, min(1.0, float(review.confidence)))
    review.reason = review.reason[:200]
    return review


_q: "queue.Queue[tuple]" = queue.Queue(maxsize=QUEUE_MAX)
_thread: threading.Thread | None = None
_stop = threading.Event()
_stats = {"enqueued": 0, "dropped": 0, "reviewed": 0, "failed": 0,
          "inflight": 0, "max_inflight": 0}


def maybe_enqueue(event_id: int, snapshot: str | None, level: int) -> bool:
    """事件落库后调用：按配置决定是否入复核队列。provider=none 时零开销。"""
    if settings.genai_provider == "none":
        return False
    if not snapshot or level < settings.genai_min_level:
        return False
    try:
        _q.put_nowait((int(event_id), str(snapshot)))
        _stats["enqueued"] += 1
        return True
    except queue.Full:
        _stats["dropped"] += 1  # 满队列丢弃+计数，绝不阻塞 pipeline
        return False


def _review_one(event_id: int, snapshot_path: str) -> None:
    _assert_loopback(settings.genai_url)
    b64 = base64.b64encode(_compress_jpeg(snapshot_path) or b"").decode()
    if not b64:
        raise GenAIError("截图读取/压缩失败")
    review = parse_review(_call_provider(b64))
    _write_back(event_id, review)


def _write_back(event_id: int, review: AIReview) -> None:
    import sqlite3

    from .config import settings as _s
    con = sqlite3.connect(str(_s.db_path), timeout=10)
    try:
        con.execute(
            "UPDATE events SET ai_review=?, ai_review_ts=datetime('now','localtime'), "
            "ai_review_model=? WHERE id=?",
            (review.model_dump_json(), f"{settings.genai_provider}:{settings.genai_model}",
             event_id))
        con.commit()
    finally:
        con.close()


def _worker_loop() -> None:
    while not _stop.is_set():
        try:
            event_id, snapshot_path = _q.get(timeout=0.5)
        except queue.Empty:
            continue
        _stats["inflight"] += 1
        _stats["max_inflight"] = max(_stats["max_inflight"], _stats["inflight"])
        try:
            _review_one(event_id, snapshot_path)
            _stats["reviewed"] += 1
        except GenAIError as e:
            _stats["failed"] += 1
            log.warning("事件 #%s 复核失败（静默降级）: %s", event_id, e)
        except Exception as e:  # noqa: BLE001  worker 绝不能死
            _stats["failed"] += 1
            log.warning("事件 #%s 复核异常: %r", event_id, e)
        finally:
            _stats["inflight"] -= 1


def start_worker() -> bool:
    """启动复核线程（provider=none 不启动）。返回是否启动。"""
    global _thread
    if _thread is not None:
        return True
    if settings.genai_provider == "none":
        return False
    if settings.genai_max_concurrency != 1:
        log.warning("GENAI_MAX_CONCURRENCY=%d 非 1：同机 GPU 竞争风险自担",
                    settings.genai_max_concurrency)
    _stop.clear()
    _thread = threading.Thread(target=_worker_loop, name="genai-review", daemon=True)
    _thread.start()
    log.info("GenAI 复核已启动：provider=%s model=%s url=%s",
             settings.genai_provider, settings.genai_model, settings.genai_url)
    return True


def stop_worker() -> None:
    global _thread
    _stop.set()
    t, _thread = _thread, None
    if t is not None:
        t.join(timeout=3.0)


def status() -> dict:
    return {"provider": settings.genai_provider, "model": settings.genai_model,
            "queue_depth": _q.qsize(), **_stats}
