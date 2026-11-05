"""启动配置校验（任务95）：只读检查，FAIL/WARN/OK 列表；strict 模式 fail-fast。

不加载 ONNX、不启动线程、不改配置/文件（临时写探针写完即删）。
"""
import os
import shutil
from pathlib import Path

from .config import settings

_MIN_MODEL_BYTES = 1_000_000


def validate_settings() -> list[dict]:
    results: list[dict] = []

    def add(level, key, message, hint=""):
        results.append({"level": level, "key": key, "message": message, "hint": hint})

    # T-104（Q-02）：绑定非回环地址时强制要求管理令牌（即使不 strict 也拒绝启动）
    host = os.environ.get("LS_HOST", "127.0.0.1").strip()
    loopback = host in ("127.0.0.1", "localhost", "::1", "")
    if not loopback and not settings.admin_token and not settings.auth_disabled:
        add("FAIL", "admin_token", f"绑定 {host} 但未设 LS_ADMIN_TOKEN",
            "首启生成令牌或显式 LS_AUTH_DISABLED=1（仅回环允许）")
    elif not settings.admin_token and not settings.auth_disabled:
        add("WARN", "admin_token", "未设管理令牌（本机默认无鉴权）",
            "Q-02：首启/升级应自动生成；源码模式可手动设 LS_ADMIN_TOKEN")
    if settings.auth_disabled:
        if loopback:
            add("WARN", "auth_disabled", "LS_AUTH_DISABLED=1 已启用（仅回环绑定生效）",
                "生产环境建议关闭")
        else:
            add("FAIL", "auth_disabled", "LS_AUTH_DISABLED=1 仅在回环绑定时允许",
                "去掉 LS_AUTH_DISABLED 并设置管理令牌")

    # 目录可写
    for key in ("data_dir", "snapshot_dir", "video_dir", "annotation_pool_dir"):
        d = getattr(settings, key)
        try:
            Path(d).mkdir(parents=True, exist_ok=True)
            probe = Path(d) / ".validate_probe"
            probe.write_text("ok", encoding="ascii")
            probe.unlink(missing_ok=True)
            add("OK", key, "可写")
        except OSError as e:
            add("FAIL", key, f"不可写: {e}", "检查目录权限/杀软拦截")

    # 模型
    ppe = settings.ppe_model or settings.model_fp32
    if not ppe:
        add("WARN", "ppe_model", "未配置实验服模型", "仅人员检测，无实验服识别")
    else:
        p = Path(str(ppe))
        if not p.exists():
            add("FAIL", "ppe_model", f"不存在: {p}", "检查路径或下载模型（docs/45）")
        elif p.stat().st_size < _MIN_MODEL_BYTES:
            add("FAIL", "ppe_model", f"过小 ({p.stat().st_size / 1e6:.1f}MB)", "模型文件可能损坏")
        else:
            add("OK", "ppe_model", f"{p.name} {p.stat().st_size / 1e6:.1f}MB")
        stem = p.with_suffix("")
        classes = stem.with_suffix(".classes.txt")
        if classes.exists() and classes.read_text(encoding="utf-8", errors="replace").strip():
            add("OK", "classes", f"{classes.name}")
        else:
            add("WARN", "classes", f"词表缺失或空: {classes.name}",
                "PPE 类别将退回 LS_PPE_CLASS_NAMES 配置")

    # Web dist
    wd = settings.web_dist
    if wd and (Path(wd) / "index.html").exists():
        add("OK", "web_dist", "index.html 存在")
    else:
        add("WARN", "web_dist", f"index.html 不存在（{wd}）", "仅 API 模式；npm run build 生成")
    if settings.genai_provider != "none":  # 任务124：VLM 复核配置
        if not settings.genai_model:
            add("WARN", "genai", "provider 已启用但未配置 LS_GENAI_MODEL",
                "设置模型名（如 qwen2.5vl:7b）后重启")
        if settings.genai_max_concurrency != 1:
            add("WARN", "genai", f"并发 {settings.genai_max_concurrency} 非 1",
                "同机 GPU 与 DML 推理竞争；24h/压测建议关闭复核")

    # go2rtc
    go2rtc = settings.go2rtc_yaml
    root = Path(__file__).resolve().parents[2]
    go2rtc_exe = root / "tools" / "go2rtc.exe"
    if not go2rtc_exe.exists():
        add("WARN", "go2rtc", f"{go2rtc_exe} 不存在", "实时画面（WebRTC）不可用")

    # 安全令牌
    at = settings.admin_token
    rt = settings.read_token
    if at and len(at) < 12:
        add("WARN", "admin_token", f"长度 {len(at)} < 12", "建议使用更长令牌")
    if at and rt and at == rt:
        add("WARN", "read_token", "与 admin_token 相同", "只读令牌形同虚设（弱隔离）")
    if rt and not at:
        add("WARN", "read_token", "已设置但 admin_token 为空——只读令牌被忽略",
            "如需保护读请同时设置 LS_ADMIN_TOKEN")

    # 磁盘
    try:
        free = shutil.disk_usage(str(settings.data_dir)).free / 1e9
        if free < settings.disk_min_free_gb:
            add("WARN", "disk_free", f"{free:.1f}GB < {settings.disk_min_free_gb}GB",
                "磁盘不足可能触发紧急清理")
        else:
            add("OK", "disk_free", f"{free:.1f}GB")
    except OSError:
        add("WARN", "disk_free", "无法读取磁盘空间")

    # DB
    db = Path(str(settings.db_path))
    if db.exists():
        import sqlite3
        try:
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            ok = con.execute("PRAGMA quick_check").fetchone()[0] == "ok"
            con.close()
            if ok:
                add("OK", "database", "quick_check ok")
            else:
                add("FAIL", "database", "quick_check 失败", "用备份恢复（scripts/backup_data.py）")
        except sqlite3.Error as e:
            add("FAIL", "database", f"打不开: {e}", "数据库损坏——用备份恢复")
    else:
        add("OK", "database", "尚不存在（首次启动自动创建）")

    return results


def fail_count(results: list[dict]) -> int:
    return sum(1 for r in results if r["level"] == "FAIL")


def warn_count(results: list[dict]) -> int:
    return sum(1 for r in results if r["level"] == "WARN")
