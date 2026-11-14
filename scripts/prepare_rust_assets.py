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
    """基础模型（yolo11n.onnx）失败即退出；PPE 模型按 Q-08 口径降级为警告
    （许可待确认不随包分发，资产源仓库尚未就绪——CI 不因它阻塞）。"""
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
    ORTDIR.mkdir(parents=True, exist_ok=True)
    if (ORTDIR / "onnxruntime.dll").is_file() and not force:
        log("onnxruntime.dll 已存在")
        return
    try:
        import onnxruntime  # noqa: F401
    except ImportError:
        log("警告：未安装 onnxruntime（pip install onnxruntime），运行时 DLL 缺失")
        return
    capi = Path(onnxruntime.__file__).resolve().parent / "capi"
    copied = 0
    for dll in capi.glob("onnxruntime*.dll"):
        shutil.copy2(dll, ORTDIR / dll.name)
        copied += 1
    log(f"onnxruntime DLL 复制 {copied} 个 <- {capi}")


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
