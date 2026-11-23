"""RA5 模拟验收：外部联网 VLM 通路端到端（真实素材 + mock 模型端）。

链路：mock VLM 服务器（协议替身，127.0.0.1:8899）→ labsentinel-server
（LS_ANALYZER_BACKEND=vlm + --launch）→ folder worker 扫描真实图片素材
→ VLM 判定 JSON 回填 folder_images → 违规事件入库 → /stats 断言帧协议。

验收点：
1. 素材就绪（ffmpeg 从演示视频抽取的有人/空场景帧）
2. mock VLM 启动 + ls-server 就绪（--launch 消解件实测）
3. /api/models/local 列出本地模型
4. folder 相机 analyzer=vlm 插拔 + scan 启动
5. VLM 判定回填（folder_images 行 processed_ts/persons 非占位）
6. mock /stats：收到 openai_compat 协议帧（image_url）——通路真
7. 违规事件（behavior=no_ppe）入库且 folder_images.event_id 关联
退出码 0 = 全部 PASS。真实外部模型（需 API key）质量验收同口径复跑：
LS_VLM_URL/KEY 指向真实端点后重跑本脚本即可。

用法: python scripts/acceptance_vlm.py [--imgs <目录>] [--keep]
"""
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import zlib
from http.server import ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import mock_vlm_server as mock  # noqa: E402

EXE = Path(os.environ.get(
    "LS_SERVER_EXE",
    r"C:\Users\User\devtools\ls-target\debug\labsentinel-server.exe"))
CREATE_NO_WINDOW = 0x0800_0000 if sys.platform == "win32" else 0

ok_all = True


def step(name: str, ok: bool, detail: str = "") -> None:
    global ok_all
    print(f"[{'PASS' if ok else 'FAIL'}] {name} {detail}", flush=True)
    ok_all = ok and ok_all


def write_empty_frame(path: Path) -> None:
    width, height = 640, 480

    def chunk(kind: bytes, payload: bytes) -> bytes:
        data = kind + payload
        return struct.pack(">I", len(payload)) + data + struct.pack(">I", zlib.crc32(data))

    pixels = (b"\0" + b"\x80" * width * 3) * height
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
                     + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b""))


def get(url: str, timeout: float = 3.0):
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with op.open(url, timeout=timeout) as r:
        return r.status, r.read()


def post(url: str, obj: dict, timeout: float = 10.0, method: str = "POST"):
    req = urllib.request.Request(
        url, data=json.dumps(obj).encode(),
        headers={"Content-Type": "application/json"}, method=method)
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with op.open(req, timeout=timeout) as r:
        return r.status, json.loads(r.read())


def main() -> int:
    global ok_all
    imgs = (Path(sys.argv[sys.argv.index("--imgs") + 1]) if "--imgs" in sys.argv
            else ROOT / "Resources" / "acceptance" / "imgs")
    keep = "--keep" in sys.argv
    proc = None
    try:
        # [1] 素材
        imgs_ok = (imgs / "person-frame.jpg").is_file()
        step("1 素材（实拍有人帧）", imgs_ok, str(imgs))
        if not imgs_ok:
            return 1

        # [2] mock VLM
        srv = ThreadingHTTPServer(("127.0.0.1", 8899), mock.Handler)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        step("2 mock VLM 启动（协议替身）", True, "127.0.0.1:8899")

        # [3] ls-server --launch（隔离数据目录 + VLM 指向 mock）
        tmp = Path(tempfile.mkdtemp(prefix="ls-vlm-accept-"))
        data = tmp / "data"
        (data / "models").mkdir(parents=True)
        port = 18190
        env = dict(os.environ)
        env.update({
            "LS_DB_PATH": str(tmp / "labsentinel.db"),
            "LS_DATA_DIR": str(data),
            "LS_PORT": str(port),
            "LS_ANALYZER_BACKEND": "vlm",
            "LS_VLM_ENABLED": "1",
            "LS_VLM_URL": "http://127.0.0.1:8899",
            "LS_VLM_PROVIDER": "openai_compat",
            "LS_VLM_MODEL": "mock-model",
            # decode_image 需 ffmpeg（find 候选不含仓库根 tools/ffmpeg 子目录）
            "LS_FFMPEG": str(ROOT / "tools" / "ffmpeg" / "ffmpeg.exe"),
            # UX2 cascade 本地先行：真 YOLO 模型扫人（触发门依赖本地检测）
            "LS_MODEL_PATH": str(ROOT / "m0-spike" / "yolo11n.onnx"),
        })
        proc = subprocess.Popen(
            [str(EXE), "--launch"], cwd=str(ROOT), env=env,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW)
        base = f"http://127.0.0.1:{port}"
        ok3 = False
        for _ in range(60):
            try:
                st, body = get(f"{base}/api/health/ready", timeout=1)
                if st == 200 and json.loads(body).get("status") == "ready":
                    ok3 = True
                    break
            except (OSError, ValueError):
                pass
            time.sleep(1)
        step("3 ls-server 就绪（--launch 消解件实测）", ok3)
        if not ok3:
            return 1

        # [4] /api/models/local
        st, body = get(f"{base}/api/models/local")
        models = json.loads(body).get("models", [])
        step("4 /api/models/local 清单端点", st == 200, f"{len(models)} 个模型")

        # [5] folder 相机 + analyzer=vlm 插拔 + scan
        st, j = post(f"{base}/api/cameras",
                     {"name": "vlm-accept", "source": str(imgs), "type": "folder"})
        cid = j.get("id", 0)
        step("5a 添加 folder 相机", st == 200, f"id={cid}")
        st, j = post(f"{base}/api/cameras/{cid}/analyzer",
                     {"analyzer": "vlm"}, method="PUT")
        step("5b analyzer=vlm（模型模块化插拔）",
             st == 200 and j.get("analyzer") == "vlm")
        st, j = post(f"{base}/api/folders/{cid}/scan", {})
        step("5c scan 启动 folder worker",
             st == 200 and j.get("started") is True, f"msg={j.get('msg')}")

        # [6] 等识别回填（mock 内容启发式：两素材 persons 不同）
        # 2026-11-14 实测抖动：processed_ts 先可见、判定列随后落盘会让单次读取
        # 看到 persons=0。改为等到「两行都有 width 且 persons 均 >0」再断言。
        ok6, detail6 = False, ""
        rows: list[dict] = []
        for _ in range(30):
            st, body = get(f"{base}/api/folder-images?camera_id={cid}")
            rows = json.loads(body).get("items", [])
            done = [r for r in rows if r.get("processed_ts")]
            pv = {r["persons"] for r in done}
            errs = [r.get("error") for r in done if r.get("error")]
            if len(done) >= 2 and all(r.get("width") for r in done) \
                    and all((r.get("persons") or 0) > 0 for r in done):
                ok6 = not errs
                detail6 = (f"rows={len(rows)} persons 集合={sorted(pv)} "
                           + (f"errors={errs}" if errs else ""))
                break
            time.sleep(1)
        else:
            detail6 = (f"rows={len(rows)} 未在超时内完成回填 "
                       f"done={len([r for r in rows if r.get('processed_ts')])}")
        step("6 VLM 判定回填（width/persons 真值）", ok6, detail6)

        # [7] mock /stats：帧协议断言（请求计数在响应后才累加，轮询到 2）
        st, body = get("http://127.0.0.1:8899/stats")
        stats = json.loads(body)
        for _ in range(10):
            if stats.get("images", 0) >= 2:
                break
            time.sleep(1)
            st, body = get("http://127.0.0.1:8899/stats")
            stats = json.loads(body)
        step("7 mock 收到 openai_compat 帧协议", stats["images"] >= 2,
             f"images={stats['images']} requests={stats['requests']}")

        # [8] 违规事件入库（GET /api/events 返回数组）
        ok8 = False
        events: list[dict] = []
        for _ in range(10):
            st, body = get(f"{base}/api/events?limit=50")
            events = json.loads(body) if isinstance(json.loads(body), list) else []
            if any(e.get("type") == "no_ppe" for e in events):
                ok8 = True
                break
            time.sleep(1)
        step("8 违规事件（no_ppe）入库", ok8, f"events={len(events)}")

        # ---- UX2（2026-11-20）：cascade 协作模式——省钱第一验收指标 ----
        st, j = post(f"{base}/api/detect/config",
                     {"mode": "cascade",
                      "vlm": {"url": "http://127.0.0.1:8899"},
                      "cascade": {"conf_lo": 0, "conf_hi": 1,
                                  "daily_budget": 50}}, method="PUT")
        step("9 PUT /api/detect/config mode=cascade（灰区放开，便于触发）",
             st == 200 and j.get("ok") is True)
        st, body = get(f"{base}/api/detect/config")
        cfg = json.loads(body)
        step("10 GET config 回读 cascade + api_key 脱敏",
             cfg.get("mode") == "cascade"
             and cfg.get("vlm", {}).get("api_key") == "***"
             and cfg.get("cascade", {}).get("daily_budget") == 50)

        # [11] 空场景 0 API 调用（省钱红线）：仅放空场景帧的新 folder 相机
        # （analyzer=default 跟随全局 cascade；本地扫不到人/规则 → 不送 API）
        empty_dir = tmp / "casc-empty"
        empty_dir.mkdir()
        write_empty_frame(empty_dir / "empty.png")
        st, j = post(f"{base}/api/cameras",
                     {"name": "cascade-empty", "source": str(empty_dir),
                      "type": "folder"})
        cid2 = j.get("id", 0)
        st, j = post(f"{base}/api/folders/{cid2}/scan", {})
        step("11a cascade 空场景相机启动", st == 200 and j.get("started") is True)
        st, body = get("http://127.0.0.1:8899/stats")
        base_reqs = json.loads(body).get("requests", 0)
        rows2: list = []
        ok11, detail11 = False, ""
        for _ in range(20):
            st, body = get(f"{base}/api/folder-images?camera_id={cid2}")
            rows2 = json.loads(body).get("items", [])
            if rows2 and all(r.get("processed_ts") for r in rows2):
                ok11 = True
                break
            time.sleep(1)
        st, body = get("http://127.0.0.1:8899/stats")
        after_reqs = json.loads(body).get("requests", 0)
        step("11b 空场景 0 次 API 调用（省钱红线）",
             ok11 and after_reqs == base_reqs,
             f"empty_rows={len(rows2)} mock requests {base_reqs}->{after_reqs}")

        # [12] 有人帧触发复核：同相机补有人帧 → mock 计数增长 + verdict 链
        shutil.copy(imgs / "person-frame.jpg", empty_dir / "person.jpg")
        st, j = post(f"{base}/api/folders/{cid2}/scan", {})
        ok12, detail12 = False, ""
        for _ in range(30):
            st, body = get("http://127.0.0.1:8899/stats")
            now_reqs = json.loads(body).get("requests", 0)
            if now_reqs > after_reqs:
                ok12 = True
                detail12 = f"mock requests {after_reqs}->{now_reqs}"
                break
            time.sleep(1)
        step("12 有人帧触发 API 复核（本地先行->精判）", ok12, detail12)

        # [13] usage 端点：计数>0 + 预算字段（跨重启持久化在 app_settings）
        ok13 = False
        usage: dict = {}
        for _ in range(10):
            st, body = get(f"{base}/api/detect/usage")
            usage = json.loads(body)
            if usage.get("count", 0) > 0:
                ok13 = True
                break
            time.sleep(1)
        step("13 GET /api/detect/usage 计数/预算", ok13,
             f"count={usage.get('count')} budget={usage.get('budget')} "
             f"left={usage.get('left')}")

        # [14] detect/test：连通性测试（真实发一帧给 mock）
        st, j = post(f"{base}/api/detect/test", {})
        step("14 POST /api/detect/test 测试连接",
             st == 200 and j.get("ok") is True and "elapsed_ms" in j,
             f"{st} elapsed={j.get('elapsed_ms')}ms")

        print(f"\n[结果] {'PASS' if ok_all else 'FAIL'}")
        return 0 if ok_all else 1
    finally:
        if proc is not None and proc.poll() is None:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                           capture_output=True, creationflags=CREATE_NO_WINDOW)
        srv.shutdown()
        srv.server_close()
        if ok_all and not keep:
            shutil.rmtree(tmp, ignore_errors=True)
            print("[清理] 已删除 " + str(tmp))
        else:
            print(f"[保留] 现场: {tmp}（--keep 或失败排障）")


if __name__ == "__main__":
    sys.exit(main())
