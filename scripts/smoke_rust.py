"""Rust 单二进制便携包冒烟（RA3，ADR-0011）：隔离解包 → 启动 → 就绪 →
API/前端断言 → 停止 → 清理。对齐 smoke_portable.py 的判定口径，但目标形态
是 labsentinel-server.exe（无嵌入式 Python，无安装器）。

用法: python scripts/smoke_rust.py --zip <labsentinel-rust-*.zip> [--timeout 90]
      python scripts/smoke_rust.py --install-dir <安装目录> [--timeout 90]
退出码 0 = 全部 PASS。失败自动保留现场（--keep 可强制保留）。
"""
import argparse
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from datetime import datetime
from pathlib import Path
from contextlib import closing

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from prepare_rust_assets import PORTABLE_MODEL_ASSETS, valid_model_asset

CREATE_NO_WINDOW = 0x0800_0000 if sys.platform == "win32" else 0
DEVNULL = subprocess.DEVNULL


def http_get(url: str, timeout: float = 2.0) -> tuple[int, bytes]:
    import urllib.request
    req = urllib.request.Request(url)
    # 回环绝不走系统代理（与 smoke_portable 同口径）
    op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with op.open(req, timeout=timeout) as r:
        return r.status, r.read()


def http_post(url: str, obj: dict, timeout: float = 3.0, *, method: str = "POST") -> tuple[int, bytes]:
    import urllib.request as _uq
    req = _uq.Request(url, data=__import__("json").dumps(obj).encode(),
                      headers={"Content-Type": "application/json"}, method=method)
    op = _uq.build_opener(_uq.ProxyHandler({}))
    with op.open(req, timeout=timeout) as r:
        return r.status, r.read()


def check_file_detection(base: str, pkg: Path, sandbox: Path, timeout: int) -> tuple[bool, str]:
    sample = ROOT / "Resources" / "acceptance" / "imgs" / "person-frame.jpg"
    source = sandbox / "photo-input"
    source.mkdir()
    shutil.copyfile(sample, source / "person.jpg")
    _, body = http_post(f"{base}/api/cameras", {
        "name": "发布验收照片", "type": "folder", "source": str(source), "autostart": True,
    })
    folder_id = json.loads(body)["id"]
    finish = time.monotonic() + timeout
    while time.monotonic() < finish:
        _, body = http_get(f"{base}/api/folder-images?camera_id={folder_id}&limit=1")
        items = json.loads(body)["items"]
        if items and items[0]["error"]:
            return False, f"照片读取失败：{items[0]['error']}"
        if items and items[0]["width"]:
            if items[0]["persons"] < 1:
                return False, "已解码的基准照片含有人，随包模型未检出人员"
            break
        _, status = http_get(f"{base}/api/cameras/{folder_id}/status")
        if json.loads(status)["status"] == "model_error":
            return False, "随包人员模型加载失败"
        time.sleep(0.5)
    else:
        return False, "有人的基准照片没有完成识别"
    _, body = http_get(f"{base}/api/events?camera_id={folder_id}&category=check")
    checks = json.loads(body)
    if len(checks) != 1 or checks[0]["type"] != "photo_check" or checks[0]["status"] != "recorded":
        return False, "未配置告警规则的正常照片检查没有独立留痕"
    event_url = f"{base}/api/events/{checks[0]['id']}"
    _, image = http_get(f"{event_url}/snapshot")
    if not image.startswith(b"\xff\xd8"):
        return False, "图片检查记录没有可预览的原图"
    _, sample_response = http_post(f"{base}/api/dataset/samples/from-event/{checks[0]['id']}", {}, timeout=20)
    if not json.loads(sample_response).get("sample", {}).get("id"):
        return False, "图片检查记录无法加入标注池"

    with closing(sqlite3.connect(sandbox / "data" / "labsentinel.db")) as db:
        with db:
            db.execute(
                "INSERT INTO app_settings(key,value,updated_ts) VALUES('ppe_model',?,datetime('now'))",
                (str(sandbox / "removed-custom-ppe.onnx"),),
            )
        try:
            _, body = http_get(f"{base}/api/models/ppe/status")
            previous = json.loads(body)
            check_photo_rules(base, sandbox, sample, timeout)
            _, body = http_get(f"{base}/api/models/ppe/status")
            after = json.loads(body)
            if after["source"] != "bundled" or after["loaded_sources"] <= previous["loaded_sources"] or not after["warning"]:
                raise ValueError("自定义文件缺失后，随包 PPE 未真实接管或状态未显示回退")
        finally:
            with db:
                db.execute("DELETE FROM app_settings WHERE key='ppe_model'")

    ffmpeg = pkg / "tools" / "ffmpeg" / "ffmpeg.exe"
    clip = sandbox / "发布验收视频.mp4"
    subprocess.run(
        [str(ffmpeg), "-hide_banner", "-loglevel", "error", "-nostdin", "-loop", "1",
         "-framerate", "12", "-i", str(sample), "-t", "3", "-vf", "scale=1280:720",
         "-c:v", "libx264", "-pix_fmt", "yuv420p", str(clip)],
        check=True, timeout=20, creationflags=CREATE_NO_WINDOW,
    )
    _, body = http_post(f"{base}/api/cameras", {
        "name": "发布验收视频", "type": "file", "source": str(clip),
    })
    video_id = json.loads(body)["id"]
    _, preview = http_get(f"{base}/api/cameras/{video_id}/snapshot?preview=1", timeout=10)
    if not preview.startswith(b"\xff\xd8") or not preview.endswith(b"\xff\xd9"):
        return False, "启动分析前无法读取视频首帧"
    http_post(f"{base}/api/cameras/{video_id}/start", {})
    finish = time.monotonic() + timeout
    while time.monotonic() < finish:
        _, body = http_get(f"{base}/api/cameras/{video_id}/status")
        state = json.loads(body)["status"]
        if state == "completed":
            break
        if state in ("error", "model_error"):
            return False, f"无规则视频检查失败，状态 {state}"
        time.sleep(0.5)
    else:
        return False, "无规则视频未完成检查"
    _, body = http_get(f"{base}/api/events?camera_id={video_id}&limit=20")
    events = json.loads(body)
    if len(events) != 1 or events[0]["type"] != "video_check" or events[0]["status"] != "recorded":
        return False, "无规则视频只应有一条检查记录"
    _, image = http_get(f"{base}/api/events/{events[0]['id']}/snapshot")
    if not events[0]["has_snapshot"] or not image.startswith(b"\xff\xd8"):
        return False, "视频检查记录没有检查截图"

    _, body = http_post(f"{base}/api/zones", {
        "camera_id": video_id, "name": "验收区域",
        "polygon": [[0, 0], [1280, 0], [1280, 720], [0, 720]],
    })
    http_post(f"{base}/api/rules", {
        "camera_id": video_id, "zone_id": json.loads(body)["id"], "behavior": "enter",
        "level": 2, "enabled": True,
    })
    http_post(f"{base}/api/cameras/{video_id}/start", {})
    finish = time.monotonic() + timeout
    state = "未知"
    while time.monotonic() < finish:
        _, body = http_get(f"{base}/api/cameras/{video_id}/status")
        state = json.loads(body)["status"]
        if state == "completed":
            _, body = http_get(f"{base}/api/events?camera_id={video_id}&limit=20")
            events = json.loads(body)
            if not any(event["camera_id"] == video_id and event["type"] == "enter" for event in events):
                return False, "视频已检查完成，但配置的进入规则未生成事件"
            _, body = http_get(f"{base}/api/events?camera_id={video_id}&category=check")
            checks = json.loads(body)
            if len(checks) != 2 or any(event["type"] != "video_check" or event["status"] != "recorded" for event in checks):
                return False, "视频检查结果没有独立留痕"
            _, body = http_get(f"{base}/api/events?camera_id={video_id}&category=alert")
            if {event["type"] for event in json.loads(body)} != {"enter"}:
                return False, "告警分类混入检查结果"
            check_failed_detection(base, pkg, sandbox, clip, timeout)
            return True, "检查留痕、视频预览、照片规则和推理失败对照通过"
        if state in ("error", "model_error"):
            return False, f"视频检查失败，状态 {state}"
        time.sleep(0.5)
    return False, f"视频未到达检查完成状态，最后状态 {state}"


def check_photo_rules(base: str, sandbox: Path, sample: Path, timeout: int) -> None:
    folder = sandbox / "photo-rule"
    folder.mkdir()
    shutil.copyfile(sample, folder / "person.jpg")
    _, body = http_post(f"{base}/api/cameras", {
        "name": "照片规则验收", "type": "folder", "source": str(folder), "autostart": False,
    })
    cid = json.loads(body)["id"]
    _, body = http_post(f"{base}/api/zones", {
        "camera_id": cid, "name": "画面外区域",
        "polygon": [[5000, 5000], [5100, 5000], [5100, 5100], [5000, 5100]],
    })
    zone = json.loads(body)["id"]
    common = {"camera_id": cid, "behavior": "occupancy_over", "threshold": 0}
    for fields in (
        {"zone_id": zone, "level": 1},
        {"days_of_week": str(datetime.now().isoweekday() % 7 + 1), "level": 1},
        {"enabled": False, "level": 1},
    ):
        http_post(f"{base}/api/rules", {**common, **fields})
    _, body = http_post(f"{base}/api/rules", {**common, "level": 3})
    rule_id = json.loads(body)["id"]
    http_post(f"{base}/api/cameras/{cid}/start", {})

    def wait_photo(name, expect_alert):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            _, body = http_get(f"{base}/api/folder-images?camera_id={cid}&limit=20")
            for row in json.loads(body)["items"]:
                if row["name"] == name and row["width"] and (row["event_id"] or not expect_alert):
                    if row["error"] or not row["persons"]:
                        raise ValueError(f"照片 {name} 未正确检出人员")
                    return
            time.sleep(0.25)
        raise ValueError(f"照片 {name} 未产生预期检查结果")

    wait_photo("person.jpg", True)
    _, body = http_post(f"{base}/api/rules/{rule_id}", {}, method="DELETE")
    deleted = json.loads(body)["policy_revision"]
    shutil.copyfile(sample, folder / "after-delete.jpg")
    http_post(f"{base}/api/folders/{cid}/scan", {})
    wait_photo("after-delete.jpg", False)
    _, body = http_post(f"{base}/api/rules", {**common, "level": 2})
    added = json.loads(body)["policy_revision"]
    shutil.copyfile(sample, folder / "after-add.jpg")
    http_post(f"{base}/api/folders/{cid}/scan", {})
    wait_photo("after-add.jpg", True)
    _, body = http_get(f"{base}/api/cameras/{cid}/status")
    state = json.loads(body)
    if state["status"] != "running" or not deleted < added <= state["applied_policy_revision"]:
        raise ValueError("运行中的照片规则未应用已保存的版本")
    _, body = http_get(f"{base}/api/events?camera_id={cid}&category=alert")
    hits = [(event["type"], event["level"]) for event in json.loads(body)]
    if hits != [("occupancy_over", 2), ("occupancy_over", 3)]:
        raise ValueError(f"照片规则区域、排班、启停、等级或热更新不符：{hits}")


def check_failed_detection(base: str, pkg: Path, sandbox: Path, clip: Path, timeout: int) -> None:
    fixture = pkg / "models" / "face_detection_yunet_2023mar.onnx"
    # 这是兼容 RGB 输入的随包人脸模型，输出不符合人员检测器契约。
    with closing(sqlite3.connect(sandbox / "data" / "labsentinel.db")) as db:
        with db:
            db.execute(
                "INSERT INTO app_settings(key,value,updated_ts) VALUES('model.active',?,datetime('now'))",
                (str(fixture),),
            )
        try:
            check_video_failure(base, clip, timeout, "error", r"已处理 0 帧.*[1-9]\d* 帧人员推理失败")
            with db:
                db.execute("UPDATE app_settings SET value=? WHERE key='model.active'", (str(sandbox / "missing-person.onnx"),))
            check_video_failure(base, clip, timeout, "model_error", "人员模型无法加载")
        finally:
            with db:
                db.execute("DELETE FROM app_settings WHERE key='model.active'")


def check_video_failure(base: str, clip: Path, timeout: int, expected: str, detail: str) -> None:
    import re

    _, body = http_post(f"{base}/api/cameras", {
        "name": f"视频失败验收-{expected}", "type": "file", "source": str(clip), "autostart": False,
    })
    cid = json.loads(body)["id"]
    http_post(f"{base}/api/cameras/{cid}/start", {})
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        _, body = http_get(f"{base}/api/cameras/{cid}/status")
        state = json.loads(body)["status"]
        if state in ("completed", "model_error", "error"):
            if state != expected:
                raise ValueError(f"视频失败预期状态 {expected}，实际 {state}")
            _, body = http_get(f"{base}/api/events?camera_id={cid}&category=check")
            events = json.loads(body)
            if len(events) != 1 or not re.search(detail, events[0]["detail"]):
                raise ValueError(f"视频未留下 {expected} 对应的检查记录")
            return
        time.sleep(0.25)
    raise ValueError(f"视频未进入 {expected} 终态")


def check_close_mask(base: str, pkg: Path, sandbox: Path, fixture: Path, timeout: int) -> None:
    """Use a known unmasked person photo to exercise both source types through the API."""
    source = sandbox / "mask-photo"
    source.mkdir()
    shutil.copyfile(fixture, source / "person.jpg")
    clip = sandbox / "mask-video.mp4"
    subprocess.run([
        str(pkg / "tools/ffmpeg/ffmpeg.exe"), "-hide_banner", "-loglevel", "error",
        "-nostdin", "-loop", "1", "-framerate", "12", "-i", str(fixture), "-t", "4",
        "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-c:v", "libx264", "-pix_fmt", "yuv420p", str(clip),
    ], check=True, timeout=20, creationflags=CREATE_NO_WINDOW)
    # Isolate the supplement from the bundled detector's own mask votes.
    http_post(f"{base}/api/models/ppe/config", {"violation_conf": .95, "wearing_conf": .95}, method="PUT")
    for enabled in (False, True):
        http_post(f"{base}/api/models/ppe/config", {"close_mask_enabled": enabled}, method="PUT")
        for source_type, path in (("folder", source), ("file", clip)):
            _, body = http_post(f"{base}/api/cameras", {
                "name": f"口罩验收-{source_type}-{enabled}", "type": source_type, "source": str(path),
            })
            camera_id = json.loads(body)["id"]
            http_post(f"{base}/api/rules", {
                "camera_id": camera_id, "zone_id": 0, "behavior": "no_mask", "level": 2, "enabled": True,
            })
            http_post(f"{base}/api/cameras/{camera_id}/start", {})
            finish = time.monotonic() + timeout
            check_type = "photo_check" if source_type == "folder" else "video_check"
            events = []
            types = set()
            while time.monotonic() < finish:
                _, body = http_get(f"{base}/api/events?camera_id={camera_id}&limit=30")
                events = json.loads(body)
                types = {event["type"] for event in events}
                if check_type in types and (not enabled or "no_mask" in types):
                    break
                time.sleep(0.5)
            if source_type == "folder":
                _, stopped = http_post(f"{base}/api/cameras/bulk", {"ids": [camera_id], "action": "stop"})
                if camera_id not in json.loads(stopped)["ok"]:
                    raise ValueError("口罩验收图片源未停止")
            if check_type not in types or ("no_mask" in types) != enabled:
                raise ValueError(f"{source_type} 启用={enabled} 事件不符：{sorted(types)}")
            if enabled:
                event_id = next(event["id"] for event in events if event["type"] == "no_mask")
                _, image = http_get(f"{base}/api/events/{event_id}/snapshot")
                if not image.startswith(b"\xff\xd8"):
                    raise ValueError(f"{source_type} 未戴口罩告警缺少截图")


def main() -> int:
    ap = argparse.ArgumentParser(description="Rust 单二进制便携包冒烟（RA3/UX3）")
    source = ap.add_mutually_exclusive_group(required=True)
    source.add_argument("--zip", type=Path)
    source.add_argument("--install-dir", type=Path)
    ap.add_argument("--timeout", type=int, default=90)
    ap.add_argument("--mask-fixture", type=Path, help="可选，已知未戴口罩的单人近景照片，用于独立验证口罩补判")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    if a.zip is not None and not a.zip.is_file():
        print(f"[FAIL] zip 不存在: {a.zip}")
        return 1
    if a.install_dir is not None and not a.install_dir.is_dir():
        print(f"[FAIL] 安装目录不存在: {a.install_dir}")
        return 1
    tmp = Path(tempfile.mkdtemp(prefix="ls-rust-smoke-"))
    ok_all = True
    proc = None
    try:
        # [1] 隔离解包
        if a.zip is not None:
            pkg = tmp / "pkg"
            with __import__("zipfile").ZipFile(a.zip) as z:
                z.extractall(pkg)
        else:
            pkg = a.install_dir.resolve()
        exe = pkg / "labsentinel-server.exe"
        if not exe.is_file():
            exe = pkg / "labsentinel-server"
        required = [exe, pkg / "tools" / "go2rtc.exe", pkg / "tools" / "go2rtc-LICENSE.txt"]
        required.extend((pkg / "LICENSE", pkg / "models" / "close-mask-LICENSE.txt"))
        missing = [p.relative_to(pkg).as_posix() for p in required if not p.is_file()]
        missing.extend(f"models/{name} 缺失或校验失败" for name in PORTABLE_MODEL_ASSETS
                       if not valid_model_asset(pkg / "models" / name))
        ok1 = not missing
        print(f"[{'PASS' if ok1 else 'FAIL'}] 1 产物文件 —— {', '.join(missing) if missing else '服务、流媒体网关及近景口罩模型齐备'}")
        ok_all &= ok1
        if not ok1:
            return 1

        # [2] 隔离环境启动（数据/DB 全在 sandbox，不污染真实 LOCALAPPDATA）
        sandbox = tmp / "sandbox"
        (sandbox / "data").mkdir(parents=True)
        port = 18180
        env = {key: value for key, value in os.environ.items()
               if not key.upper().startswith(("LS_", "ORT_"))}
        env.update({
            "LS_DB_PATH": str(sandbox / "data" / "labsentinel.db"),
            "LS_DATA_DIR": str(sandbox / "data"),
            "LS_PORT": str(port),
            "LS_HOST": "127.0.0.1",
            "LS_WEB_DIST": str(pkg / "web"),
            "LS_ADMIN_TOKEN": "",
            "LS_LAUNCHER_NO_BROWSER": "1",
        })
        proc = subprocess.Popen(
            [str(exe)], cwd=str(pkg), env=env,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW)
        base = f"http://127.0.0.1:{port}"

        # [3] 就绪轮询（/api/health/ready）
        ok3, last = False, ""
        deadline = time.time() + a.timeout
        while time.time() < deadline:
            if proc.poll() is not None:
                last = f"进程提前退出 rc={proc.returncode}"
                break
            try:
                st, body = http_get(f"{base}/api/health/ready", timeout=1.0)
                if st == 200 and json.loads(body).get("status") == "ready":
                    ok3 = True
                    break
            except (OSError, ValueError) as e:
                last = f"{type(e).__name__}"
            time.sleep(1)
        readiness = "ready" if ok3 else (last or "timeout")
        print(f"[{'PASS' if ok3 else 'FAIL'}] 3 就绪 —— {readiness}")
        ok_all &= ok3

        # [4] /api/system：backend=rust + version 精确匹配包内 VERSION + PPE 状态可达
        ok4, detail = False, ""
        want_ver = (pkg / "VERSION").read_text("ascii").strip() if (pkg / "VERSION").is_file() else ""
        try:
            st, body = http_get(f"{base}/api/system", timeout=3)
            j = json.loads(body)
            got_ver = j.get("version") or ""
            ver_ok = (got_ver == want_ver) if want_ver else bool(got_ver)
            st_ppe, body_ppe = http_get(f"{base}/api/models/ppe/status", timeout=3)
            j_ppe = json.loads(body_ppe) if st_ppe == 200 else {}
            ppe_ok = (st_ppe == 200 and isinstance(j_ppe.get("enabled"), bool)
                      and j_ppe.get("coat_confirmation") is True
                      and j_ppe.get("close_mask_available") is True
                      and j_ppe.get("close_mask_enabled") is False)
            if ppe_ok:
                for enabled in (True, False):
                    _, changed = http_post(f"{base}/api/models/ppe/config", {"close_mask_enabled": enabled}, method="PUT")
                    _, status = http_get(f"{base}/api/models/ppe/status")
                    ppe_ok &= (json.loads(changed).get("close_mask_enabled") is enabled
                               and json.loads(status).get("close_mask_enabled") is enabled)
            ok4 = st == 200 and j.get("backend") == "rust" and ver_ok and ppe_ok
            detail = f"backend={j.get('backend')} version={got_ver} (want={want_ver}) ppe_enabled={j_ppe.get('enabled')}"
        except (OSError, ValueError) as e:
            detail = repr(e)
        print(f"[{'PASS' if ok4 else 'FAIL'}] 4 /api/system + PPE —— {detail}")
        ok_all &= ok4

        # [5] 前端 SPA（index + immutable 资产头抽验）
        ok5, detail5 = False, ""
        try:
            st, html = http_get(f"{base}/", timeout=3)
            marker = b'id="app"' in html
            asset_ok = True
            import re as _re
            assets = _re.findall(r'(?:src|href)="(/assets/[^"]+)"', html.decode("utf-8", "replace"))
            if assets:
                st2, _ = http_get(f"{base}{assets[0]}", timeout=3)
                asset_ok = st2 == 200
            ok5 = st == 200 and marker and asset_ok
            detail5 = f"根标记={'命中' if marker else '缺失'} 资产 {assets[0] if assets else '无'}"
        except (OSError, ValueError) as e:
            detail5 = repr(e)
        print(f"[{'PASS' if ok5 else 'FAIL'}] 5 前端 —— {detail5}")
        ok_all &= ok5

        try:
            if ok3:
                db_path = sandbox / "data" / "labsentinel.db"
                with closing(sqlite3.connect(db_path)) as db, db:
                    db.execute(
                        "INSERT INTO app_settings(key,value,updated_ts) VALUES('runtime.ep','cpu',datetime('now')) "
                        "ON CONFLICT(key) DO UPDATE SET value=excluded.value"
                    )
                detection_ok, detection_detail = check_file_detection(base, pkg, sandbox, a.timeout)
            else:
                detection_ok, detection_detail = False, "服务未就绪"
        except (OSError, ValueError, KeyError, subprocess.SubprocessError, sqlite3.Error) as error:
            detection_ok, detection_detail = False, str(error)
        print(f"[{'PASS' if detection_ok else 'FAIL'}] 6 随包照片与视频检测 —— {detection_detail}")
        ok_all &= detection_ok

        if detection_ok and a.mask_fixture is not None:
            try:
                check_close_mask(base, pkg, sandbox, a.mask_fixture.resolve(strict=True), a.timeout)
                print("[PASS] 近景口罩对照 —— 启用后，照片与视频均生成带截图的未戴口罩告警")
            except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
                print(f"[FAIL] 近景口罩对照 —— {error}")
                ok_all = False

        # [6] UX3 GUI 形态：无控制台子系统的 exe 照常就绪（步骤 3 即证——release
        # 二进制 windows_subsystem="windows"；此处断言进程仍存活且 API 可响应），
        # 加 shutdown API 优雅退出（托盘「退出」同一函数；0.5s 内进程消失）
        ok6, detail6 = False, ""
        try:
            st, _ = http_get(f"{base}/api/health/live", timeout=2)
            alive = proc.poll() is None
            if st == 200 and alive:
                st2, body2 = http_post(f"{base}/api/system/shutdown", {}, timeout=3)
                exited = False
                for _ in range(10):
                    if proc.poll() is not None:
                        exited = True
                        break
                    time.sleep(0.5)
                rc = proc.returncode
                ok6 = exited and rc == 0
                detail6 = f"shutdown={st2} 进程退出={'是' if exited else '否'} rc={rc}"
            else:
                detail6 = f"live={st} alive={alive}"
        except (OSError, ValueError) as e:
            detail6 = repr(e)
        print(f"[{'PASS' if ok6 else 'FAIL'}] 7 GUI 形态 + shutdown 优雅退出 —— {detail6}")
        ok_all &= ok6
    finally:
        if proc is not None and proc.poll() is None:
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"],
                           capture_output=True, creationflags=CREATE_NO_WINDOW)
        if a.keep:
            print(f"[保留] 现场: {tmp}（--keep）")
        else:
            try:
                shutil.rmtree(tmp)
                print("[清理] 已删除 " + str(tmp))
            except OSError as error:
                print(f"[FAIL] 无法清理测试目录 {tmp}: {error}")
                ok_all = False
    print(f"[结果] {'PASS' if ok_all else 'FAIL'}")
    return 0 if ok_all else 1


if __name__ == "__main__":
    sys.exit(main())
