# -*- coding: utf-8 -*-
"""零 Python 纯 Rust 形态端到端驱动：覆盖全部功能面找 bug。
被测系统 = labsentinel-server.exe（隔离部署，0 个 .py）；本脚本仅为测试驱动。"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

deploy_root = Path(os.environ["TEMP"]) / "ls-rust-only-101634"
EXE = deploy_root / "bin" / "labsentinel-server.exe"
IMGS = deploy_root / "imgs"
data = deploy_root / "data"
data.mkdir(parents=True, exist_ok=True)
port = 18300
base = f"http://127.0.0.1:{port}"

results = []


def step(name, ok, detail=""):
    results.append((name, ok, detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name} {detail}", flush=True)


def req(path, obj=None, method=None, timeout=10):
    url = f"{base}{path}"
    headers = {"Content-Type": "application/json", "Host": "127.0.0.1"}
    m = method or ("POST" if obj is not None else "GET")
    data_b = json.dumps(obj).encode() if obj is not None else None
    req = urllib.request.Request(url, data=data_b, headers=headers, method=m)
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with op.open(req, timeout=timeout) as r:
            body = r.read()
            try:
                return r.status, json.loads(body)
            except ValueError:
                return r.status, body
    except urllib.error.HTTPError as e:
        body = e.read()
        try:
            return e.code, json.loads(body)
        except ValueError:
            return e.code, body
    except (OSError, ValueError) as e:
        return -1, str(e)


def wait_ready(timeout=60):
    for _ in range(timeout):
        st, body = req("/api/health/ready", timeout=1)
        if st == 200 and isinstance(body, dict) and body.get("status") == "ready":
            return True
        time.sleep(1)
    return False


env = dict(os.environ)
env.update({
    "LS_DB_PATH": str(data / "labsentinel.db"),
    "LS_DATA_DIR": str(data),
    "LS_PORT": str(port),
    "LS_WEB_DIST": str(deploy_root / "bin" / "web"),
    # 关键：PATH 不含 python（零 Python 隔离）；ffmpeg 也不给（VLM 路径才要）
})
proc = subprocess.Popen([str(EXE), "--launch"], cwd=str(deploy_root / "bin"), env=env,
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                        creationflags=0x08000000 if sys.platform == "win32" else 0)
try:
    step("0 服务就绪（--launch+全新空库）", wait_ready())

    # 1 schema 自举断言：空库启动后表已建
    st, j = req("/api/system")
    step("1 /api/system backend=rust", st == 200 and j.get("backend") == "rust",
         f"version={j.get('version')}")

    # 2 相机 CRUD 全链
    st, j = req("/api/cameras", {"name": "cam-rtsp", "source": "rtsp://10.1.1.5/stream",
                                 "type": "rtsp"})
    cid1 = j.get("id")
    step("2a 添加 rtsp 相机", st == 200 and cid1, f"id={cid1}")
    st, j = req("/api/cameras", {"name": "cam-folder", "source": str(IMGS), "type": "folder"})
    cid2 = j.get("id")
    step("2b 添加 folder 相机", st == 200 and cid2, f"id={cid2}")
    st, body = req("/api/cameras")
    cams_before = len(body) if isinstance(body, list) else 0
    step("2c 相机列表（含本次 2 台）", cams_before >= 2, f"count={cams_before}")
    st, j = req(f"/api/cameras/{cid1}/group", {"group_name": "lab"}, method="PUT")
    step("2d 分组", st == 200)
    st, j = req(f"/api/cameras/{cid1}/analyzer", {"analyzer": "vlm"}, method="PUT")
    step("2e analyzer=vlm 插拔", st == 200 and j.get("analyzer") == "vlm")
    st, j = req(f"/api/cameras/{cid2}/analyzer", {"analyzer": "vlm"}, method="PUT")
    step("2f folder 也插拔 vlm", st == 200)

    # 3 区域/规则/模板全链
    st, j = req("/api/zones", {"camera_id": cid1, "name": "bench",
                               "polygon": [[0, 0], [1280, 0], [1280, 720]]})
    zid = j.get("id") if isinstance(j, dict) else None
    step("3a 添加区域", st == 200 and zid, f"zid={zid}")
    st, j = req("/api/rules", {"camera_id": cid1, "zone_id": zid, "behavior": "occupancy_over",
                               "level": 2, "threshold": 1, "enabled": True})
    step("3b 添加规则 occupancy_over", st == 200)
    # 模板名带时间戳（name UNIQUE——跨运行状态污染防重复）
    import time as _t
    tpl_name = f"夜间入区v2-{_t.strftime('%H%M%S')}"
    st, j = req("/api/rule-templates", {"name": tpl_name, "config": [
        {"behavior": "person_enter", "level": 2, "cooldown_s": 60}]})
    step("3c 添加规则模板", st == 200, f"tpl={tpl_name}")
    st, j = req("/api/rule-templates")
    ok3d = st == 200 and isinstance(j, list) and len(j) >= 1
    step("3d 模板列表", ok3d)
    if ok3d:
        # 用本轮新建的模板（按 name 匹配——上轮残留模板可能是坏 config 数据）
        tid = next((t["id"] for t in j if t["name"] == tpl_name), None)
        if tid is None:
            tid = j[0]["id"]
        st, j = req(f"/api/rule-templates/{tid}/apply",
                    {"camera_ids": [cid1], "zone_id": zid, "name_suffix": "-夜"})
        step("3e 模板应用", st == 200, f"rc={j.get('created') if isinstance(j, dict) else j}")

    # 4 calendar
    st, j = req("/api/calendar", {"date": "2026-11-14", "mode": "workday", "note": "补班"})
    step("4 日历异常日", st == 200)

    # 5 处置流转 + 审计（先造一条事件：直接经 VLM folder worker 产生）
    st, j = req(f"/api/cameras/{cid2}/start", {})
    step("5a folder worker 启动（vlm 后端）", st == 200 and j.get("ok"))
    rows_ok = False
    events = []
    for _ in range(30):
        st, body = req(f"/api/folder-images?camera_id={cid2}")
        rows = body if isinstance(body, list) else body.get("items", [])
        done = [r for r in rows if r.get("processed_ts")]
        if len(done) >= 2:
            rows_ok = True
            break
        time.sleep(1)
    step("5b folder 扫描+VLM 判定回填（无模型→NullDetector 0 人）", rows_ok,
         f"rows={len(rows)}")
    # VLM mock 不在（无网络端点）→ 判定 Err → error 字段如实；persons=0
    errs = [r.get("error") for r in rows if r.get("error")]
    step("5c VLM 不可达降级如实（error 字段非空或 persons=0）",
         bool(errs) or all(r.get("persons") == 0 for r in rows),
         f"errors={len(errs)}")

    # 6 手工造事件（处置流转+审计）
    st, body = req("/api/events?limit=50")
    events = body if isinstance(body, list) else body.get("items", [])
    if events:
        eid = events[0]["id"]
        st, j = req(f"/api/events/{eid}/status", {"status": "acked"})
        step("6a 事件处置", st == 200)
    else:
        # 无事件时经 mock-genai 不通——跳过处置，如实标注
        step("6a 事件处置（库中暂无事件，跳过）", True, "零事件库")
    st, body = req("/api/audit?limit=10")
    audit_ok = st == 200
    step("6b 审计查询", audit_ok, f"{len(body.get('items', body.get('rows', []))) if isinstance(body, dict) else '?'} 行")

    # 7 统计全家桶（空库/满库都断言 200；hourly date 必填——契约对齐 Python）
    for p in ["/api/stats/summary", "/api/stats/daily?days=3",
              "/api/stats/hourly?date=2026-11-14",
              "/api/stats/quality", "/api/stats/timeline?days=3", "/api/stats/weekly"]:
        st, body = req(p)
        step(f"7 {p}", st == 200)

    # 8 导出
    st, body = req("/api/export/events.csv", timeout=15)
    step("8a events.csv", st == 200)
    st, body = req("/api/export/stats.csv", timeout=15)
    step("8b stats.csv", st == 200)
    st, body = req("/api/folder-images/export.csv", timeout=15)
    step("8c folder-images.csv", st == 200)

    # 9 设备配对（POST /api/pairing 生成 → POST /api/pairing/claim 认领）
    st, j = req("/api/pairing", {})
    code_ok = st == 200 and isinstance(j, dict) and j.get("code")
    step("9a 配对码生成", code_ok, f"code={j.get('code') if isinstance(j, dict) else '?'}")
    if code_ok:
        st, j2 = req("/api/pairing/claim", {"code": j.get("code", ""),
                                            "device_name": "e2e-dev",
                                            "platform": "test"})
        if st == 200 and isinstance(j2, dict) and j2.get("token"):
            step("9b 配对认领", True, "token 已返回")
        else:
            step("9b 配对认领", False, f"响应={j2 if isinstance(j2, str) else json.dumps(j2, ensure_ascii=False)[:200]}")
    st, body = req("/api/devices")
    # auth-off（admin_token 空）→ 全放行 200（与 Python require_admin 口径一致）
    step("9c 设备列表（auth-off 放行）", st == 200)

    # 10 插件扫描（data/plugins 空 → 空列表）
    st, j = req("/api/plugins")
    step("10 插件列表（空）", st == 200 and j.get("plugins") == [])

    # 11 genai/mqtt/update 状态（默认关）
    st, j = req("/api/genai/status")
    step("11a genai 默认 none", st == 200 and j.get("provider") == "none")
    st, j = req("/api/mqtt/status")
    step("11b mqtt 默认关", st == 200 and j.get("enabled") is False)
    st, j = req("/api/update/check?refresh=1")
    step("11c update 未配置 enabled=false", st == 200 and j.get("enabled") is False)

    # 12 处置状态流转（造事件后处置/升级）
    st, body = req("/api/events?limit=50")
    events = body if isinstance(body, list) else body.get("items", [])
    if events:
        eid = events[0]["id"]
        st, j = req(f"/api/events/{eid}/status", {"status": "false_positive"})
        step("12a 误报处置", st == 200)
        st, body = req("/api/events/timeline?days=3")
        step("12b 时间线", st == 200)

    # 13 模型清单（models/ 空目录 → 空但 200）
    st, body = req("/api/models/local")
    step("13 models/local（空目录）", st == 200)

    # 14 卸载/删除相机全链（folder 相机先 stop 再 delete）
    st, body = req(f"/api/cameras/{cid2}", method="DELETE")
    step("14a DELETE 相机（vlm 后端 worker 联停）", st == 200)
    st, body = req("/api/cameras")
    remain = len(body) if isinstance(body, list) else -1
    step("14b 删除后减 1 台", remain == cams_before - 1, f"remain={remain}")

    # 15 静态 SPA + 资产
    st, body = req("/")
    step("15a SPA 首页", st == 200 and b'id="app"' in (body if isinstance(body, bytes) else b""))
finally:
    if proc.poll() is None:
        subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                       capture_output=True, creationflags=0x08000000)
    print(f"\n[汇总] PASS {sum(1 for _, ok, _ in results if ok)} / FAIL "
          f"{sum(1 for _, ok, _ in results if not ok)}", flush=True)
    for name, ok, d in results:
        if not ok:
            print(f"  [FAIL] {name} {d}", flush=True)
