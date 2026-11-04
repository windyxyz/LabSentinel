"""导出 OpenAPI 快照（任务101）：机器可读契约，防手写文档漂移。

用法:
  python scripts/export_openapi.py                    # 生成 docs/openapi.json
  python scripts/export_openapi.py --out <path>       # 指定输出
  python scripts/export_openapi.py --check            # 与现有快照比对，不一致 exit 2（CI/RC 用）

只导入 app.main:app（不启动 lifespan/不碰 DB/模型）。
"""
import argparse
import json
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "docs" / "openapi.json"


def build_openapi() -> dict:
    sys.path.insert(0, str(ROOT / "server"))
    from app.main import app  # 导入即注册全部路由；不跑 lifespan
    spec = app.openapi()
    _inject_examples(spec)
    return spec


# 关键端点示例（任务101；示例值全部为演示数据，不含真实密钥/地址）
_ERROR = {"detail": "资源不存在", "request_id": "0123abcd", "code": "not_found"}
_EXAMPLES = {
    ("post", "/api/cameras"): {"request": {
        "name": "示例摄像头", "source": "rtsp://user:pass@192.168.1.10:554/Streaming/Channels/102",
        "type": "rtsp", "autostart": False, "group_name": "二楼东"}},
    ("get", "/api/cameras/{cid}/status"): {"responses": {"200": {
        "id": 1, "name": "示例摄像头", "type": "rtsp", "status": "running",
        "fps": 15.0, "frame_age_s": 0.3, "reconnect_count": 0, "restart_count": 0,
        "supervisor_state": "running"}}},
    ("get", "/api/health/ready"): {"responses": {"200": {
        "status": "ok", "checks": {"db": "ok", "model": "ok", "disk": "ok"}}}},
    ("get", "/api/events"): {"responses": {"200": [
        {"id": 123, "ts": "2026-11-04T09:00:00", "camera_id": 1, "type": "enter_no_ppe",
         "level": 3, "track_id": 0, "detail": "示例事件", "has_snapshot": True,
         "has_video": False, "status": "new", "ai_review": None, "ai_verdict": None}]}},
    ("post", "/api/events/bulk-status"): {"request": {
        "ids": [123, 124], "status": "acked"}},
    ("get", "/api/events/{eid}/evidence.zip"): {"responses": {"404": _ERROR}},
}


def _inject_examples(spec: dict) -> None:
    for (method, path), ex in _EXAMPLES.items():
        node = spec.get("paths", {}).get(path, {}).get(method)
        if node is None:
            continue
        if "request" in ex and "requestBody" in node:
            node["requestBody"].setdefault("content", {}).setdefault(
                "application/json", {})["example"] = ex["request"]
        for code, payload in ex.get("responses", {}).items():
            resp = node.get("responses", {}).get(code)
            if resp is not None:
                resp.setdefault("content", {}).setdefault(
                    "application/json", {})["example"] = payload


def main() -> int:
    ap = argparse.ArgumentParser(description="导出 OpenAPI 快照（任务101）")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--check", action="store_true",
                    help="生成到临时文件与现有快照比对，不一致 exit 2")
    a = ap.parse_args()
    try:
        spec = build_openapi()
    except Exception as e:  # noqa: BLE001
        print(f"[失败] 无法导入 app.main:app：{e!r}")
        return 1
    text = json.dumps(spec, ensure_ascii=False, indent=2)
    if not a.check:
        a.out.parent.mkdir(parents=True, exist_ok=True)
        a.out.write_text(text + "\n", encoding="utf-8")
        print(f"[OK] {a.out}（paths={len(spec.get('paths', {}))}，"
              f"{len(text) / 1024:.0f}KB）")
        return 0
    if not a.out.exists():
        print(f"[失败] 快照不存在: {a.out}（先不带 --check 生成一次）")
        return 2
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False,
                                     encoding="utf-8") as tf:
        tf.write(text + "\n")  # 与非 check 路径同尾换行，否则恒差 1 字节
        tmp = Path(tf.name)
    try:
        if tmp.read_text(encoding="utf-8") == a.out.read_text(encoding="utf-8"):
            print(f"[OK] 快照与当前代码一致（{a.out}）")
            return 0
        print(f"[FAIL] 快照过期：{a.out} 与 app.openapi() 不一致——"
              f"运行 python scripts/export_openapi.py 更新后提交")
        return 2
    finally:
        tmp.unlink(missing_ok=True)


if __name__ == "__main__":
    sys.exit(main())
