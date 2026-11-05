"""T-103（S-02 / P0）：摄像头 source 严格校验 + go2rtc YAML 注入防护。

所有摄像头入口（API add_camera / 配置导入 / 启动迁移检查）统一走 validate_source；
go2rtc 侧 source_for 只消费已校验的 source，build_yaml 用 JSON 字符串转义
（JSON 是 YAML 子集，零依赖，引号/换行无法注入任意配置）。
"""
import json
import os
import re
from pathlib import Path

_RTSP = re.compile(r"^rtsps?://[^\s\"'#\x00-\x1f]+$", re.IGNORECASE)
VIDEO_EXTS = {".mp4", ".mkv", ".avi", ".mov", ".ts", ".flv"}

#: 系统目录黑名单（图片文件夹允许根，T-111；默认拒绝用户主目录之外的系统位置）
SYSTEM_DIR_NAMES = {"windows", "program files", "program files (x86)", "programdata",
                    "system", "etc", "usr", "bin", "sbin", "library"}


class SourceError(ValueError):
    """source 不合规（调用方转 400）。"""


def validate_source(type_: str, source: str, *, allow_missing: bool = False) -> str:
    """按 type 白名单校验；返回规范化后的 source。

    - rtsp：必须 rtsps?://，禁止 # / 空白 / 引号 / 控制字符（挡 exec: / ffmpeg:#raw / YAML 注入）
    - file：存在的视频文件（扩展名白名单），绝对路径，禁止 # / 控制字符；allow_missing=True
      时只做语法检查（配置导入：目标机路径尚未存在，存在性在启动时报告）
    - usb：0-99 纯数字
    - folder：交给 check_folder_root（T-111）；这里只做长度/控制字符/换行检查
    """
    raw = source or ""
    if any(c in raw for c in ("\n", "\r", "\x00")):
        raise SourceError("source 含控制字符（换行/NUL）")
    s = raw.strip()
    if not s or len(s) > 1024:
        raise SourceError("source 为空或过长（>1024）")
    if type_ == "rtsp":
        if not _RTSP.match(s):
            raise SourceError("rtsp 源必须是 rtsp:// 或 rtsps:// 地址，且不能含空白/引号/#")
        return s
    if type_ == "file":
        if "#" in s or "\"" in s or "'" in s:
            raise SourceError("file 源路径不能含 # 或引号")
        p = Path(s).expanduser()
        if p.suffix.lower() not in VIDEO_EXTS:
            raise SourceError("file 源必须是视频文件（mp4/mkv/avi/mov/ts/flv）")
        # 存在性不在此强制：源码模式常用占位/待挂载视频源（x.mp4、跨机器路径），
        # worker 启动时以 camera.status=error 明确报告；这里只堵注入面与扩展名误用。
        # 运维可通过 doctor/启动迁移检查（startup_issues）看到缺失文件。
        return str(p.resolve())
    if type_ == "usb":
        if not re.fullmatch(r"\d{1,2}", s):
            raise SourceError("usb 源必须是 0-99 的设备号")
        return s
    if type_ == "folder":
        return s
    raise SourceError(f"未知 type: {type_}")


def check_folder_root(source: str, allowed_roots: str = "") -> str:
    """T-111：图片文件夹允许根。allowed_roots 为空=允许任意目录但拒绝系统目录黑名单；
    非空时目录必须位于其一（commonpath 判断，resolve 防符号链接逃逸）。"""
    s = (source or "").strip()
    if not s or len(s) > 1024 or any(c in s for c in ("\n", "\r", "\x00")):
        raise SourceError("图片文件夹路径为空/过长/含控制字符")
    p = Path(s).expanduser()
    if not p.is_dir():
        raise SourceError("图片文件夹不存在或不是目录")
    real = p.resolve()
    parts_lower = {q.name.lower() for q in real.parents}
    if parts_lower & SYSTEM_DIR_NAMES:
        raise SourceError("图片文件夹不能位于系统目录（Windows/Program Files/AppData/System 等）")
    if os.name == "nt":
        appdata = Path(os.environ.get("APPDATA", ""))
        local = Path(os.environ.get("LOCALAPPDATA", ""))
        for base in (appdata, local):
            if base and str(base) and (real == base or base in real.parents):
                raise SourceError("图片文件夹不能位于用户 AppData 配置目录")
    roots = [r.strip() for r in (allowed_roots or "").replace(";", ",").split(",") if r.strip()]
    if roots:
        for r in roots:
            rp = Path(r).expanduser().resolve()
            try:
                import os as _os
                if _os.path.commonpath([str(real), str(rp)]) == str(rp):
                    return str(real)
            except ValueError:
                continue
        raise SourceError("图片文件夹不在 LS_FOLDER_ALLOWED_ROOTS 允许范围内")
    return str(real)


def yaml_stream_entry(cam_id: int | str, src: str) -> str:
    """YAML 流条目：JSON 字符串转义（合法 YAML 子集，零依赖，彻底挡引号/换行注入）。"""
    return f"  {json.dumps(str(cam_id))}: {json.dumps(str(src))}"


def startup_issues(cameras: list[dict]) -> list[str]:
    """启动迁移检查：已存摄像头中不合规/文件缺失的逐条报告（不自动删除/改写数据）。"""
    out = []
    for cam in cameras:
        try:
            validate_source(cam.get("type", ""), cam.get("source", ""))
        except SourceError as e:
            out.append(f"摄像头 #{cam.get('id')} ({cam.get('name')}): {e}")
        else:
            if cam.get("type") == "file" and not Path(cam.get("source", "")).expanduser().is_file():
                out.append(f"摄像头 #{cam.get('id')} ({cam.get('name')}): 视频文件不存在（worker 启动将报 error）")
    return out
