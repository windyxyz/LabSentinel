# -*- coding: utf-8 -*-
"""RA4.2：准备 Rust 发行包所需资产（模型 / ffmpeg / ffprobe / onnxruntime）。

用途：本地与 CI 的 `scripts/build_rust.py --strict --zip` 前置步骤。
幂等：目标已存在即跳过；下载走 `scripts/fetch_assets.py`（哈希锁校验）。
不含任何 Python 运行时依赖——只服务构建，不进入分发包。

用法：
    python scripts/prepare_rust_assets.py [--force]
"""
import argparse
import gzip
import shutil
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import fetch_assets  # noqa: E402

MODELS = ROOT / "m0-spike"
FFDIR = ROOT / "tools" / "ffmpeg"
ORTDIR = ROOT / "tools" / "onnxruntime"
MODEL_ASSETS = ("yolo11n.onnx", "yolo11n_ppe.onnx", "yolo11n_ppe.classes.txt")


def log(msg: str) -> None:
    print(f"[prepare] {msg}", flush=True)


def ensure_models(force: bool) -> None:
    """模型准备：基础模型失败即退出。PPE 模型自 0.3.1 起随仓库/发行包分发
    （m0-spike 已入库），此处仅在文件缺失时尝试锁文件下载，失败仍降级警告；
    发行构建由 build_rust.py --strict 强制校验 PPE 是否存在。"""
    MODELS.mkdir(parents=True, exist_ok=True)
    for name in MODEL_ASSETS:
        out = MODELS / name
        if out.is_file() and not force:
            log(f"模型已存在: {out.name}")
            continue
        rc = fetch_assets.download(name, out)
        if rc != 0:
            if name == "yolo11n.onnx":
                raise SystemExit(f"[失败] 基础模型下载失败: {name}")
            log(f"警告: {name} 下载失败（许可/资产源未就绪，Q-08）——"
                "发行包不含实验服模型，用户可在网页端按需确认许可后下载")



def ensure_ffmpeg(force: bool) -> None:
    FFDIR.mkdir(parents=True, exist_ok=True)
    ff = FFDIR / "ffmpeg.exe"
    if not ff.is_file() or force:
        raw = FFDIR / "ffmpeg-win32-x64.gz"
        if fetch_assets.download("ffmpeg-win32-x64.gz", raw) != 0:
            raise SystemExit("[失败] ffmpeg 下载失败")
        with gzip.open(raw, "rb") as src, ff.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        raw.unlink(missing_ok=True)
        log(f"ffmpeg 解压完成: {ff}")
    else:
        log("ffmpeg 已存在")
    fprobe = FFDIR / "ffprobe.exe"
    if not fprobe.is_file() or force:
        tgz = FFDIR / "ffprobe-static.tgz"
        if fetch_assets.download("ffprobe-static.tgz", tgz) != 0:
            log("警告：ffprobe 下载失败（仅影响摄像头探测，不影响取流）")
            return
        with tarfile.open(tgz, "r:gz") as tar:
            member = None
            for m in tar.getmembers():
                n = m.name.replace("\\", "/").lower()
                if n.endswith("bin/win32/x64/ffprobe.exe"):
                    member = m
                    break
            if member is None:
                log("警告：ffprobe 压缩包内未找到 win32-x64 二进制")
            else:
                f = tar.extractfile(member)
                if f is not None:
                    with fprobe.open("wb") as dst:
                        shutil.copyfileobj(f, dst)
                    log(f"ffprobe 解压完成: {fprobe}")
        tgz.unlink(missing_ok=True)
    else:
        log("ffprobe 已存在")


def ensure_ort(force: bool) -> None:
    """onnxruntime 运行时 DLL——**DirectML 版**（UX 2026-11-21：GPU 推理默认开）。

    DML = DX12 统一层：N 卡 / A 卡 / Intel i 卡 / 各核显全部走同一 DLL（微软
    官方 nuget Microsoft.ML.OnnxRuntime.DirectML；哈希钉死）。CPU-only 机器同
    一 DLL 也可跑 CPU EP。依赖系统 DirectML.dll（Win10 1709+ 自带）。
    """
    ORTDIR.mkdir(parents=True, exist_ok=True)
    target = ORTDIR / "onnxruntime.dll"
    WANT_SHA256 = "903c92c54acc57caa77d44d4c856c829eb82f5cf6755b47367a8153bd3fabeb6"
    NUGET_URL = "https://www.nuget.org/api/v2/package/Microsoft.ML.OnnxRuntime.DirectML/1.20.1"
    if target.is_file():
        import hashlib
        got = hashlib.sha256(target.read_bytes()).hexdigest()
        if got == WANT_SHA256:
            log("onnxruntime.dll 已存在（DirectML 版，哈希一致）")
            return
        log(f"onnxruntime.dll 哈希不符（旧 CPU 版?）——重取 DML 版: {got[:16]}…")
    # 下载 nuget（zip）→ 解压 win-x64 native onnxruntime.dll → 校验哈希 → 落盘
    import hashlib
    import tempfile
    import urllib.request
    import zipfile
    with tempfile.TemporaryDirectory() as td:
        pkg = Path(td) / "ortdml.nupkg"
        log(f"下载 onnxruntime-directml nuget ← {NUGET_URL}")
        op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with op.open(NUGET_URL, timeout=600) as r, pkg.open("wb") as dst:
            shutil.copyfileobj(r, dst)
        with zipfile.ZipFile(pkg) as z:
            member = "runtimes/win-x64/native/onnxruntime.dll"
            data = z.read(member)
        got = hashlib.sha256(data).hexdigest()
        if got != WANT_SHA256:
            raise SystemExit(f"[失败] onnxruntime.dll 哈希不符: {got}")
        target.write_bytes(data)
        log(f"onnxruntime.dll（DirectML 版 {len(data)/1e6:.1f}MB）已落盘，SHA256 一致")


def main() -> int:
    ap = argparse.ArgumentParser(description="准备 Rust 发行包资产")
    ap.add_argument("--force", action="store_true", help="重新下载/复制全部资产")
    a = ap.parse_args()
    ensure_models(a.force)
    ensure_ffmpeg(a.force)
    ensure_ort(a.force)
    return 0


if __name__ == "__main__":
    sys.exit(main())
