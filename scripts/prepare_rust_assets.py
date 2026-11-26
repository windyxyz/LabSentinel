# -*- coding: utf-8 -*-
"""准备 Rust 发行包所需资产（模型 / ffmpeg / ffprobe / onnxruntime / go2rtc）。

用途：本地与 CI 的 `scripts/build_rust.py --strict --zip` 前置步骤。
幂等：已验证的模型可复用。下载走 `scripts/fetch_assets.py`（哈希锁校验）。
不含任何 Python 运行时依赖——只服务构建，不进入分发包。

用法：
    python scripts/prepare_rust_assets.py [--force]
"""
import argparse
import gzip
import shutil
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import fetch_assets  # noqa: E402

MODELS = ROOT / "models"
FFDIR = ROOT / "tools" / "ffmpeg"
ORTDIR = ROOT / "tools" / "onnxruntime"
MODEL_ASSETS = ("yolo11n_ppe.onnx", "yolo11n_ppe.classes.txt", "labcoat-confirm.onnx")
CLOSE_MASK_ASSETS = ("face_detection_yunet_2023mar.onnx", "mask_detector.onnx")
PUBLIC_MODEL_ASSETS = (*CLOSE_MASK_ASSETS, "yolo26n-pose.onnx", "yolo26n.onnx")
PORTABLE_MODEL_ASSETS = ("yolo11n.onnx", *MODEL_ASSETS, *PUBLIC_MODEL_ASSETS)
GO2RTC = ROOT / "tools" / "go2rtc.exe"
GO2RTC_ARCHIVE = "go2rtc_win64.zip"


def log(msg: str) -> None:
    print(f"[prepare] {msg}", flush=True)


def valid_go2rtc(path: Path) -> bool:
    spec = fetch_assets.load_lock()[GO2RTC_ARCHIVE]
    return path.is_file() and fetch_assets.verify(path, spec["binary_sha256"], spec["binary_size"])


def valid_model_asset(path: Path) -> bool:
    spec = fetch_assets.load_lock()[path.name]
    sha = spec.get("prepared_sha256", spec["sha256"])
    size = spec.get("prepared_size", spec["size"])
    return path.is_file() and fetch_assets.verify(path, sha, size)


def ensure_go2rtc(force: bool) -> None:
    if not force and valid_go2rtc(GO2RTC):
        log("go2rtc.exe 已存在，SHA256 一致")
        return
    GO2RTC.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=GO2RTC.parent) as work:
        archive = Path(work) / GO2RTC_ARCHIVE
        if fetch_assets.download(GO2RTC_ARCHIVE, archive) != 0:
            raise SystemExit("[失败] go2rtc 下载失败")
        binary = Path(work) / GO2RTC.name
        with zipfile.ZipFile(archive) as package, package.open(GO2RTC.name) as src, binary.open("wb") as dst:
            shutil.copyfileobj(src, dst)
        if not valid_go2rtc(binary):
            raise SystemExit("[失败] go2rtc.exe 哈希或大小不符")
        binary.replace(GO2RTC)
    log("go2rtc.exe 已验证并就绪")


def prepare_person_model(out: Path, force: bool) -> None:
    """将签入锁文件的上游模型转换到随包运行库支持的 opset，验证转换后的字节。"""
    spec = fetch_assets.load_lock()[out.name]
    if not force and valid_model_asset(out):
        log(f"{out.name} 已转换且哈希一致")
        return
    try:
        import onnx
    except ImportError as exc:
        raise SystemExit("请在构建环境安装 onnx==1.17.0，以转换随包人员模型") from exc
    with tempfile.TemporaryDirectory(dir=out.parent) as work:
        original = out
        if force or not out.is_file() or not fetch_assets.verify(out, spec["sha256"], spec["size"]):
            original = Path(work) / out.name
            if fetch_assets.download(out.name, original) != 0:
                raise SystemExit("[失败] 人员模型下载失败")
        converted = onnx.version_converter.convert_version(onnx.load(original), spec["prepared_opset"])
        onnx.checker.check_model(converted, full_check=True)
        temp_model = Path(work) / "converted.onnx"
        temp_model.write_bytes(converted.SerializeToString(deterministic=True))
        if not fetch_assets.verify(temp_model, spec["prepared_sha256"], spec["prepared_size"]):
            raise SystemExit("[失败] 转换后的人员模型 SHA256 或大小不符")
        temp_model.replace(out)
    log(f"{out.name} 已转换并校验为 opset {spec['prepared_opset']}")


def ensure_models(force: bool) -> None:
    """校验仓库内的实验服模型并准备其余随包权重。"""
    MODELS.mkdir(parents=True, exist_ok=True)
    prepare_person_model(MODELS / "yolo11n.onnx", force)
    for name in PUBLIC_MODEL_ASSETS:
        out = MODELS / name
        if not force and valid_model_asset(out):
            log(f"{name} 已存在，SHA256 一致")
            continue
        if fetch_assets.download(name, out) != 0:
            raise SystemExit(f"[失败] 随包模型准备失败: {name}")
    for name in MODEL_ASSETS:
        if not valid_model_asset(MODELS / name):
            raise SystemExit(f"[失败] 仓库内实验服模型缺失或不匹配: {name}")
        log(f"{name} 已存在，SHA256 一致")



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
    import urllib.request
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
    ensure_go2rtc(a.force)
    return 0


if __name__ == "__main__":
    sys.exit(main())
