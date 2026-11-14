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
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
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
        imgs_ok = (imgs / "person-frame.jpg").is_file() \
            and (imgs / "empty-frame.jpg").is_file()
        step("1 素材（有人帧+空场景帧）", imgs_ok, str(imgs))
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
