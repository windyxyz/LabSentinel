"""人脸比对模块（CompreFace REST 集成，默认关闭）。

合规红线（依据《人脸识别技术应用安全管理办法》，详见 docs/20-research/00-synthesis.md §7）：
- 功能默认关闭（LS_FACE_ENABLED=false），开启需改配置重启，启动日志打印合规提示
- 不落人脸图片：管线只向 CompreFace 发帧，本地只存 subject 名与置信度
- 注册必须携带 consent=true，同意记录（subject, ts）落本地 SQLite face_consents 表
- 提供一键清除（DELETE /api/face/persons 连同同意记录）
"""
import time
from urllib.parse import quote

import cv2
import httpx

from .config import settings


class FaceClient:
    """CompreFace recognition REST 客户端。网络失败一律降级返回，不抛出、不阻塞主管线。"""

    def __init__(self, base_url: str | None = None, api_key: str | None = None,
                 timeout: float = 5.0, transport: httpx.BaseTransport | None = None):
        self.base_url = (base_url or settings.compreface_url).rstrip("/")
        self.api_key = api_key or settings.compreface_api_key
        self.timeout = timeout
        self._transport = transport  # 单测注入 MockTransport

    @property
    def configured(self) -> bool:
        return bool(self.api_key)

    def _client(self) -> httpx.Client:
        kwargs = {"timeout": self.timeout}
        if self._transport is not None:  # 测试路径
            kwargs["transport"] = self._transport
        return httpx.Client(**kwargs)

    def _headers(self) -> dict:
        return {"x-api-key": self.api_key}

    # ---- 1:N 识别 ----
    def recognize(self, image_bytes: bytes) -> dict:
        """返回 {"status": "ok"|"error", "subject", "similarity"}。
        ok + subject=None = 检出人脸但相似度低于阈值（陌生人）；
        error = 服务不可用等，调用方必须跳过识别、不得据此报 stranger（降级红线）。"""
        if not self.configured:
            return {"status": "error", "subject": None, "similarity": None}
        try:
            with self._client() as c:
                r = c.post(f"{self.base_url}/recognition/faces", params={"limit": 1},
                           headers=self._headers(),
                           files={"file": ("frame.jpg", image_bytes, "image/jpeg")})
            r.raise_for_status()
            for item in r.json().get("result") or []:
                faces = item.get("faces") or []
                if faces:
                    f = faces[0]
                    sim = float(f.get("similarity") or 0)
                    if sim >= settings.face_threshold and f.get("subject"):
                        return {"status": "ok", "subject": f["subject"], "similarity": sim}
                    return {"status": "ok", "subject": None, "similarity": sim}
            return {"status": "ok", "subject": None, "similarity": None}
        except Exception as e:  # noqa: BLE001
            print(f"[face] recognize failed: {e}")
            return {"status": "error", "subject": None, "similarity": None}

    # ---- 人脸库管理 ----
    def add_person(self, subject: str, image_bytes: bytes) -> None:
        """上传样例照注册人员。失败抛异常（由 API 层转 502）。"""
        with self._client() as c:
            r = c.post(f"{self.base_url}/recognition/faces", params={"subject": subject},
                       headers=self._headers(),
                       files={"file": ("photo.jpg", image_bytes, "image/jpeg")})
            r.raise_for_status()

    def list_persons(self) -> list[str]:
        """人员名列表。CompreFace 各版本返回结构有差异，做兼容解析。"""
        with self._client() as c:
            r = c.get(f"{self.base_url}/recognition/subjects", headers=self._headers())
            r.raise_for_status()
        data = r.json()
        subs = data.get("subjects") if isinstance(data, dict) else data
        names = []
        for s in subs or []:
            if isinstance(s, str):
                names.append(s)
            elif isinstance(s, dict):
                names.append(s.get("subject") or s.get("name"))
        return [n for n in names if n]

    def delete_person(self, subject: str) -> None:
        with self._client() as c:
            r = c.delete(f"{self.base_url}/recognition/subjects/{quote(subject, safe='')}",
                         headers=self._headers())
            r.raise_for_status()


class FaceGate:
    """管线侧人脸识别门控：功能开关 + 单 track 冷却 + 失败降级。

    check() 返回："known:<名字>" / "unknown" / None。
    None = 功能未开启、冷却期内、或服务异常（降级跳过，绝不误报 stranger）。
    """

    def __init__(self, client: FaceClient | None = None):
        self.client = client or FaceClient()
        self.enabled = settings.face_enabled and self.client.configured
        self._last: dict[tuple, float] = {}  # (camera_id, track_id) -> monotonic

    def check(self, camera_id: int, track_id: int, frame, box) -> str | None:
        if not self.enabled:
            return None
        key = (camera_id, track_id)
        now = time.monotonic()
        if now - self._last.get(key, -1e9) < settings.face_cool_s:
            return None
        self._last[key] = now
        # B-21（T-306 顺带）：_last 有界——每 1000 次调用清理过期键（冷却窗 ×10 之外的）
        if len(self._last) >= 1000:
            horizon = max(60.0, float(settings.face_cool_s) * 10)
            for k in [k for k, t in self._last.items() if now - t > horizon]:
                self._last.pop(k, None)
        x1, y1, x2, y2 = (int(max(0, v)) for v in box)
        crop = frame[y1:y2, x1:x2]
        if crop.size == 0:
            return None
        ok, buf = cv2.imencode(".jpg", crop)
        if not ok:
            return None
        r = self.client.recognize(buf.tobytes())
        if r["status"] != "ok":
            return None
        if r["subject"]:
            return f"known:{r['subject']}"
        return "unknown"
