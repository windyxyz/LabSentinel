# -*- coding: utf-8 -*-
"""LabSentinel 端到端冒烟脚本（任务22）：起服务前提下的主链路自动化验证。

黑盒：全程走 HTTP API，不 import 服务端内部模块；不启动/停止服务进程——
请先用 scripts/start-all.bat（或手动 uvicorn）把服务跑起来。

用法（m0-venv）:
    python tools/e2e_smoke.py                       # 默认 http://127.0.0.1:8000
    python tools/e2e_smoke.py --base-url http://127.0.0.1:8001 --timeout 60 --keep
    python tools/e2e_smoke.py --help

流程：前置检查 → 注册文件摄像头(autostart=false) → 画安全区 → 加规则 → 启动 →
轮询等待 enter 事件 → 校验字段/截图/计数 → 清理（--keep 保留调试）。
退出码 0=PASS，1=FAIL。

⚠️ 训练/压测运行期间不要执行本脚本（会触发 DML 推理，违反"重负载一次只跑一个"铁律）。
"""
import argparse
import sys
import time
from datetime import datetime

import httpx

VIDEO = r"D:\User\Desktop\TOOLS\监控识别\m0-spike\data\lab-walk-24076.mp4"
ZONE_POLYGON = [[520, 180], [1180, 180], [1180, 620], [520, 620]]


def step(name, ok, detail=""):
    mark = "PASS" if ok else "FAIL"
    print(f"[{mark}] {name}" + (f" — {detail}" if detail else ""))
    return (name, ok, detail)  # 存元组供摘要复述；裸 bool 会让摘要解包崩（10-28 实测）


def main():
    ap = argparse.ArgumentParser(description="LabSentinel 端到端冒烟（黑盒，服务须已在运行）")
    ap.add_argument("--base-url", default="http://127.0.0.1:8000")
    ap.add_argument("--timeout", type=int, default=40, help="等待事件的最长秒数（默认 40）")
    ap.add_argument("--keep", action="store_true", help="保留测试摄像头/安全区/规则（调试用）")
    args = ap.parse_args()

    base = args.base_url.rstrip("/")
    c = httpx.Client(base_url=base, timeout=15)
    results = []

    # 1. 前置检查
    try:
        r = c.get("/api/system")
        r.raise_for_status()
        backend = r.json().get("backend", "?")
        results.append(step("前置检查 /api/system", True, f"backend={backend}"))
    except Exception as e:  # noqa: BLE001
        print(f"[FAIL] 前置检查：服务不可达（{base}）——请先启动服务（scripts\\start-all.bat）\n       {e}")
        return 1

    cam_id = zone_id = None
    try:
        # 2. 注册文件摄像头（不自动启动）
        r = c.post("/api/cameras", json={
            "name": f"e2e-smoke-{int(time.time())}", "source": VIDEO,
            "type": "file", "autostart": False})
        r.raise_for_status()
        cam_id = r.json()["id"]
        results.append(step("注册测试摄像头", True, f"id={cam_id}（autostart=false）"))

        # 3. 安全区
        r = c.post("/api/zones", json={"camera_id": cam_id, "name": "e2e-smoke-zone",
                                       "polygon": ZONE_POLYGON})
        r.raise_for_status()
        zone_id = r.json()["id"]
        results.append(step("创建安全区", True, f"id={zone_id}"))

        # 4. 规则
        for behavior, level in (("enter", 2), ("enter_no_ppe", 3)):
            r = c.post("/api/rules", json={"camera_id": cam_id, "zone_id": zone_id,
                                           "behavior": behavior, "level": level,
                                           "cooldown_s": 5})
            r.raise_for_status()
        results.append(step("添加规则 enter(L2)/enter_no_ppe(L3)", True))

        # 5. 启动 + 轮询事件
        r = c.post(f"/api/cameras/{cam_id}/start")
        r.raise_for_status()
        print(f"[....] 启动摄像头，轮询事件（最长 {args.timeout}s；DirectML 初始化需十几秒）...")
        enter_events = []
        t0 = time.monotonic()
        while time.monotonic() - t0 < args.timeout:
            evs = c.get("/api/events", params={"camera_id": cam_id, "limit": 50}).json()
            enter_events = [e for e in evs if e["type"] in ("enter", "enter_no_ppe")]
            if enter_events:
                break
            time.sleep(2)
        results.append(step("等待事件产生", bool(enter_events),
                            f"{len(enter_events)} 条（{time.monotonic()-t0:.0f}s）"))

        # 6. 字段完整性
        fields_ok = bool(enter_events) and all(
            all(k in e for k in ("ts", "type", "level", "detail", "has_snapshot"))
            for e in enter_events)
        pair_ok = any(e["type"] == "enter" for e in enter_events)
        results.append(step("事件字段完整（ts/type/level/detail/has_snapshot）", fields_ok))
        results.append(step("enter 事件存在（enter_no_ppe 成对）", pair_ok))

        # 7. 截图 + 计数一致性
        snap_ok = False
        if enter_events:
            eid = enter_events[0]["id"]
            r = c.get(f"/api/events/{eid}/snapshot")
            snap_ok = (r.status_code == 200
                       and r.headers.get("content-type", "").startswith("image/jpeg"))
        results.append(step("事件截图 image/jpeg", snap_ok))
        cnt = c.get("/api/events/count", params={"camera_id": cam_id}).json()["count"]
        rows = c.get("/api/events", params={"camera_id": cam_id, "limit": 200}).json()
        results.append(step("events/count 与列表行数一致", cnt == len(rows),
                            f"count={cnt} rows={len(rows)}"))
    finally:
        # 8. 清理（--keep 保留）
        if cam_id and not args.keep:
            try:
                c.delete(f"/api/cameras/{cam_id}")  # 连带删除 zone/rule
                results.append(step("清理测试摄像头（连带安全区/规则）", True))
            except Exception as e:  # noqa: BLE001
                results.append(step("清理测试摄像头", False, str(e)))
        elif args.keep:
            results.append(step("清理跳过（--keep）", True, f"camera_id={cam_id}"))

    # 摘要
    print("\n===== 冒烟摘要 =====")
    for name, ok, detail in results:
        print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
    all_ok = all(ok for _, ok, _ in results)
    print(f"===== 结果：{'PASS' if all_ok else 'FAIL'} =====")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
