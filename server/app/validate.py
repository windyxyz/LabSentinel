"""启动配置校验（任务95）：只读检查，FAIL/WARN/OK 列表；strict 模式 fail-fast。

不加载 ONNX、不启动线程、不改配置/文件（临时写探针写完即删）。

R2-102/N-03：
  - 绑定地址一律读 settings.host（pydantic-settings 同时读环境变量与 .env），
    不再读 os.environ["LS_HOST"]（那样会漏掉 .env 里的值）。
  - 安全类 FAIL 用 level="FATAL"：无论 LS_STRICT_CONFIG 与否都拒绝启动。
  - 运行时另有 _enforce_bind()（main.py）按 uvicorn 实际绑定地址兜底。
"""
import shutil
from pathlib import Path

from .config import settings

_MIN_MODEL_BYTES = 1_000_000


def is_loopback_host(host: str) -> bool:
    """Single definition shared by validate/api/main (R2-102)."""
    return host.strip() in ("127.0.0.1", "localhost", "::1", "")


def validate_settings() -> list[dict]:
    results: list[dict] = []

    def add(level, key, message, hint=""):
        results.append({"level": level, "key": key, "message": message, "hint": hint})

    # T-104（Q-02）+ R2-102：非回环绑定必须有管理令牌、不允许逃生口。
    # FATAL = 无条件拒绝启动（不再依赖 LS_STRICT_CONFIG）。
    host = settings.host
    loopback = is_loopback_host(host)
    if not loopback and not settings.admin_token and not settings.auth_disabled:
        add("FATAL", "admin_token", f"绑定 {host} 但未设 LS_ADMIN_TOKEN",
            "首启生成令牌或改用回环绑定（这是安全底线，不能只用 WARN）")
    elif not settings.admin_token and not settings.auth_disabled:
        add("WARN", "admin_token", "未设管理令牌（本机默认无鉴权）",
            "Q-02：首启/升级应自动生成；源码模式可手动设 LS_ADMIN_TOKEN")
    # Q-03 第 3 条：绑定非回环地址时强制开启读保护（启动校验写入，
    # 避免"开了管理令牌但实时事件流/读接口仍裸奔"）。仅改进程内 settings，
    # 不写 config.env（用户本机配置最小 diff）。
    if not loopback and not settings.auth_read_protected:
        settings.auth_read_protected = True
        add("WARN", "auth_read_protected",
            f"绑定 {host}（非回环）→ 已强制开启读保护",
            "显式设置 LS_AUTH_READ_PROTECTED=1 可消除本提示")
    if settings.auth_disabled:
        if loopback:
            add("WARN", "auth_disabled", "LS_AUTH_DISABLED=1 已启用（仅回环绑定生效）",
                "生产环境建议关闭")
        else:
            add("FATAL", "auth_disabled", "LS_AUTH_DISABLED=1 仅在回环绑定时允许",
                "去掉 LS_AUTH_DISABLED 并设置管理令牌")
    # R2-102: TrustedHostMiddleware would 400 every LAN request if trusted_hosts
    # only contains loopback names. Warn (not fatal: the admin may use a proxy).
    if not loopback:
        hosts = [h.strip().lower() for h in settings.trusted_hosts.split(",") if h.strip()]
        if hosts and all(is_loopback_host(h) for h in hosts):
            add("WARN", "trusted_hosts",
                f"绑定 {host} 但 Host 白名单只含回环名，局域网请求会被 400",
                "把可达 IP/主机名加入 LS_TRUSTED_HOSTS")

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
    return sum(1 for r in results if r["level"] in ("FAIL", "FATAL"))


def fatal_count(results: list[dict]) -> int:
    """R2-102: security FAILs that must block startup regardless of strict_config."""
    return sum(1 for r in results if r["level"] == "FATAL")


def warn_count(results: list[dict]) -> int:
    return sum(1 for r in results if r["level"] == "WARN")
