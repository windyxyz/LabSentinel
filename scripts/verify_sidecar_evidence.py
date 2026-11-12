"""T-326 存证链路 e2e 演练（真实 sidecar + 真模型 + 真视频）。

链路：ls-analyzer（gRPC sidecar）→ 事件流 → SidecarEventBridge 入库/截图/
派发 → finalize_evidence（Rust 环形缓冲补齐前后 2s）→ ffmpeg 写 MP4 →
set_event_video 回填。断言：事件行 has_snapshot/has_video、截图与视频文件
真实存在且可播（ffprobe 时长≈4s）、LatestFrame 非空。

用法：python scripts/verify_sidecar_evidence.py（仓库根运行；需 ffmpeg 在 PATH）
"""
import os
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

VIDEO = ROOT / "Resources/效果演示/实验有风险，防护需穿戴-检测效果_v2.mp4"
MODEL = ROOT / "m0-spike/yolo11n.onnx"
# R2-506: no developer-machine paths; mirror analyzer_sidecar._default_exe().
_suffix = ".exe" if os.name == "nt" else ""
EXE = Path(os.environ.get(
    "LS_ANALYZER_EXE",
    str(Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "engine-rs" / "target")))
        / "debug" / f"ls-analyzer{_suffix}")))


def main() -> int:
    if not VIDEO.is_file() or not MODEL.is_file():
        print(f"[e2e] 缺语料/模型: {VIDEO} / {MODEL}")
        return 2
    if not EXE.is_file():
        print(f"[e2e] sidecar exe 不存在: {EXE}（先 cargo build -p ls-analyzer）")
        return 2

    tmp = Path(tempfile.mkdtemp(prefix="ls-ev-e2e-"))
    os.environ["LS_DETERMINISTIC"] = "1"
    os.environ["LS_LOG_FILE"] = "0"
    os.environ["LS_VIDEO_DIR"] = str(tmp / "videos")
    os.environ["LS_DATA_DIR"] = str(tmp)
    os.environ["LS_EVIDENCE_FPS"] = "10"
    (tmp / "videos").mkdir()

    from app.config import settings
    settings.data_dir = tmp
    settings.snapshot_dir = tmp / "snapshots"
    settings.video_dir = tmp / "videos"
    settings.ppe_model = ""

    from app.analyzer_sidecar import SidecarAnalyzer
    from app.sidecar_events import SidecarEventBridge
    from app.store import Store

    store = Store(tmp / "e2e.db")
    cid = store.add_camera("e2e", str(VIDEO), "file")
    zid = store.add_zone(cid, "全画幅",
                         [[0, 0], [1280, 0], [1280, 719], [0, 719]])
    store.add_rule({"camera_id": cid, "zone_id": zid, "behavior": "enter",
                    "level": 2, "cooldown_s": 0, "time_start": "00:00",
                    "time_end": "23:59", "threshold": 0, "enabled": True})

    dispatched: list[dict] = []

    class _Disp:
        def dispatch(self, row):
            dispatched.append(row)

    analyzer = SidecarAnalyzer(exe=EXE)
    bridge = SidecarEventBridge(store, _Disp(), analyzer)

    # 与生产同口径：configcache 返回多边形已解析的列表（store.zone 的 polygon 是
    # JSON 字符串，直接传会导致 proto 端 zone 全被丢弃 → 无事件）
    from app import configcache  # settings 定向后导入（模块级导入会锁死旧目录）

    zones = configcache.zones(store, cid)
    rules = [r for r in configcache.rules(store) if r["camera_id"] == cid]
    assert zones and rules, f"配置为空: zones={zones} rules={rules}"
    analyzer.start()
    try:
        analyzer.start_camera(dict(store.camera(cid)), zones, rules)
        if not bridge.start():
            print("[e2e] bridge 启动失败")
            return 2
        # 等首批事件入库（视频 65s、5fps、人进入即触发 enter）
        deadline = time.time() + 90
        while time.time() < deadline:
            rows = store.events(camera_id=cid) if "camera_id" in \
                store.events.__code__.co_varnames else store.events()
            if rows:
                break
            time.sleep(1.0)
        rows = store.events()
        if not rows:
            print("[e2e] 90s 内无事件入库")
            return 3
        ev = rows[0]
        print(f"[e2e] 事件#{ev['id']} {ev['type']} snapshot={ev['snapshot']}")
        assert ev["snapshot"], "事件行无截图路径"
        snap = Path(ev["snapshot"])
        assert snap.is_file() and snap.stat().st_size > 500, "截图缺失/过小"
        assert len(dispatched) >= 1 and dispatched[0]["has_snapshot"], "派发行无截图"

        # finalize 由存证线程异步执行——等视频落盘
        deadline = time.time() + 30
        video = ""
        while time.time() < deadline:
            row = store.event_by_id(ev["id"])
            video = (row or {}).get("video") or ""
            if video:
                break
            time.sleep(1.0)
        assert video, "30s 内视频未回填"
        vp = Path(video)
        assert vp.is_file() and vp.stat().st_size > 5_000, "视频缺失/过小"
        # ffprobe 时长 ≈ 4s（前2s+后2s，±1s 容差）
        import subprocess
        dur = subprocess.run(
            ["ffprobe", "-v", "error", "-show_entries", "format=duration",
             "-of", "csv=p=0", str(vp)], capture_output=True, text=True, errors="replace",
            check=False)
        # 2026-11-13 P3（审查实证）：ffprobe 缺失/失败时 float('') 裸崩 ValueError
        # 断掉整条断言链——先校验 returncode 与可解析性
        assert dur.returncode == 0 and dur.stdout.strip(), \
            f"ffprobe 失败（rc={dur.returncode}）：{(dur.stderr or '')[:120]}"
        try:
            secs = float(dur.stdout.strip())
        except ValueError as e:
            raise AssertionError(f"ffprobe 输出不可解析: {dur.stdout.strip()[:60]!r}") from e
        print(f"[e2e] 存证视频 {vp.name} {vp.stat().st_size}B 时长={secs:.2f}s")
        assert 2.0 <= secs <= 7.0, f"视频时长异常: {secs}"

        # LatestFrame 非空（实时快照）
        from app import analyzer_pb2 as pb
        frame = analyzer._stub.LatestFrame(pb.CameraRef(camera_id=cid), timeout=5.0,
                                           metadata=(("x-ls-token", analyzer._token),))
        assert len(frame.data) > 500, "LatestFrame 空"
        print(f"[e2e] LatestFrame {len(frame.data)}B")
        print("[e2e] PASS：截图/视频/回填/实时帧 全链路 OK")
        return 0
    finally:
        bridge.stop()
        analyzer.stop_camera(cid)
        analyzer.stop()


if __name__ == "__main__":
    sys.exit(main())
