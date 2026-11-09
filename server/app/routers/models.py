"""Q-08 / T-406：实验服（PPE）模型按需下载——许可确认 + SHA256 校验。

Q-08：权重许可确认前不随包；首启/设置页显示数据集名称与许可说明，
用户显式确认后从发布资产下载（assets.lock.json 哈希校验），未下载时以
"仅人员检测"模式运行（settings.ppe_model 为空即可）。
"""
from __future__ import annotations

import json
import os
from pathlib import Path

from fastapi import APIRouter, Depends, HTTPException

from . import _context as ctx
from ..config import settings
from ..models import BaseModel

router = APIRouter()

# 与 scripts/assets.lock.json 的资产名一致（哈希/URL 单一来源在锁文件）
_ASSET_ONNX = "yolo11n_ppe.onnx"
_ASSET_CLASSES = "yolo11n_ppe.classes.txt"
# routers/models.py → app/routers → app → server → 仓库根
_LOCK = Path(__file__).resolve().parents[3] / "scripts" / "assets.lock.json"

LICENSE_NOTICE = (
    "实验服模型权重由 Roboflow 训练数据集派生，具体许可仍待确认（Q-08）。"
    "下载即表示你已阅读并接受该数据集的用途限制；许可确认后我们会随包分发清晰授权的自有权重。"
)


def _asset(name: str) -> dict:
    try:
        return json.loads(_LOCK.read_text(encoding="utf-8"))["assets"][name]
    except (OSError, ValueError, KeyError) as e:
        raise HTTPException(503, f"资产锁文件不可用: {e}")


def _allowed_hosts() -> set:
    """从锁文件收集允许的下载域名（下载 URL 必须逐字来自锁文件，防篡改外链）。"""
    import urllib.parse
    hosts = set()
    try:
        assets = json.loads(_LOCK.read_text(encoding="utf-8"))["assets"]
    except (OSError, ValueError):
        return hosts
    for spec in assets.values():
        for u in list(spec.get("urls", [])) + list(spec.get("mirror_urls", [])):
            try:
                p = urllib.parse.urlsplit(u)
                if p.scheme == "https" and p.hostname:
                    hosts.add(p.hostname.lower())
            except ValueError:
                pass
    return hosts


def _check_url(url: str, allowed: set) -> str:
    """SSRF 防护：https + 锁文件域名白名单 + 无凭证/查询串/片段。"""
    import urllib.parse
    p = urllib.parse.urlsplit(url)
    if p.scheme != "https" or (p.hostname or "").lower() not in allowed:
        raise ValueError(f"URL 不在锁文件域名白名单: {url}")
    if p.username or p.password or p.query or p.fragment:
        raise ValueError("URL 不允许携带凭证/查询串/片段")
    return url


def _models_dir() -> Path:
    # 与便携包模板一致：data_dir 同级的 models/；源码模式落 models/
    return Path(settings.data_dir).parent / "models"


def _ppe_paths() -> tuple[Path, Path]:
    d = _models_dir()
    return d / _ASSET_ONNX, d / _ASSET_CLASSES


def _sha256_ok(path: Path, spec: dict) -> bool:
    import hashlib
    if not path.is_file() or path.stat().st_size != spec["size"]:
        return False
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest() == spec["sha256"].lower()


def _status(onnx: Path, classes: Path) -> dict:
    spec_onnx, spec_cls = _asset(_ASSET_ONNX), _asset(_ASSET_CLASSES)
    installed = _sha256_ok(onnx, spec_onnx) and _sha256_ok(classes, spec_cls)
    return {
        "installed": installed,
        "mode": "ppe" if installed else "person_only",
        "model_path": str(onnx) if installed else None,
        "classes_path": str(classes) if installed else None,
        "license_notice": LICENSE_NOTICE,
        "license_confirmed_required": True,
        "sha256": spec_onnx["sha256"],
        "size_bytes": spec_onnx["size"],
        "urls": spec_onnx["urls"],
        # 运行时是否生效（需重启/热切换使 settings.ppe_model 指向新文件）
        "active": bool(settings.ppe_model) and installed,
    }


class PpeDownloadBody(BaseModel):
    """许可确认（extra=forbid：不接受额外字段）。"""
    model_config = {"extra": "forbid"}
    confirm_license: bool


@router.get("/models/ppe/status", dependencies=[Depends(ctx.read_guard)])
def ppe_status():
    onnx, classes = _ppe_paths()
    return _status(onnx, classes)


@router.post("/models/ppe/download", dependencies=[Depends(ctx.admin_guard)])
def ppe_download(body: PpeDownloadBody):
    """用户确认许可后下载两个资产；逐源 SHA256 校验，失败不留残件。"""
    if not body.confirm_license:
        raise HTTPException(400, "必须先确认实验服模型的数据集许可说明")
    out_onnx, out_cls = _ppe_paths()
    out_onnx.parent.mkdir(parents=True, exist_ok=True)
    _download_all(((_ASSET_ONNX, out_onnx), (_ASSET_CLASSES, out_cls)))
    # 写入进程内 settings（重启后由 config.env/设置页持久化；不碰用户配置文件）
    settings.ppe_model = str(out_onnx)
    os.environ["LS_PPE_MODEL"] = str(out_onnx)
    return {"ok": True, **_status(out_onnx, out_cls),
            "note": "已生效；重启服务后如需持久生效请保存配置"}


def _download_all(specs) -> None:
    """逐源下载 + 白名单校验 + SHA256 校验（任一资产失败抛 502）。"""
    import urllib.request
    allowed = _allowed_hosts()
    for name, out in specs:
        spec = _asset(name)
        if _sha256_ok(out, spec):
            continue  # 已装且哈希正确，跳过重复下载
        ok = False
        for url in spec["urls"]:
            try:
                _check_url(url, allowed)  # SSRF 防护：仅锁文件白名单域名
            except ValueError:
                continue
            part = out.with_suffix(out.suffix + ".part")
            try:
                with urllib.request.urlopen(url, timeout=60) as r, part.open("wb") as w:
                    while True:
                        chunk = r.read(1 << 20)
                        if not chunk:
                            break
                        w.write(chunk)
                if _sha256_ok(part, spec):
                    part.replace(out)
                    ok = True
                    break
            except (OSError, ValueError):
                pass
            finally:
                part.unlink(missing_ok=True)
        if not ok:
            raise HTTPException(502, f"资产下载/校验失败: {name}（所有源均不可用或哈希不匹配）")
