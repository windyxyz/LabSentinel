"""T-104（Q-02）：首启/升级自动生成管理令牌。

- 新装首启：生成 LS_ADMIN_TOKEN（secrets.token_urlsafe(24)）写入 config.env，
  并写 data/FIRST_RUN_TOKEN.txt（仅首次，提示用户保存后由 UI 引导删除）。
- 升级：安装器检测旧 config.env 无令牌 → 自动生成并在升级完成页显示一次。
- 逃生口 LS_AUTH_DISABLED=1：只在绑定回环地址时生效，每次启动记审计日志。
"""
import logging
import secrets
from pathlib import Path

log = logging.getLogger("app.auth_setup")


def ensure_admin_token(config_env: Path, data_dir: Path) -> str | None:
    """确保 config.env 有管理令牌；返回新生成的令牌（仅首次/升级补生成时），否则 None。

    只动 LS_ADMIN_TOKEN 一行，用户其余配置逐字保留（用户工作风格：最小 diff）。
    """
    lines = config_env.read_text(encoding="utf-8").splitlines() if config_env.exists() else []
    for line in lines:
        if line.strip().startswith("LS_ADMIN_TOKEN=") and line.split("=", 1)[1].strip():
            return None  # 已有令牌，逐字不动
    token = secrets.token_urlsafe(24)
    new_lines = [l for l in lines if not l.strip().startswith("LS_ADMIN_TOKEN=")]
    new_lines.append(f"LS_ADMIN_TOKEN={token}")
    config_env.parent.mkdir(parents=True, exist_ok=True)
    config_env.write_text("\n".join(new_lines) + "\n", encoding="utf-8")
    data_dir.mkdir(parents=True, exist_ok=True)
    (data_dir / "FIRST_RUN_TOKEN.txt").write_text(
        "LabSentinel 管理令牌（只显示一次，请复制保存后删除本文件）：\n" + token + "\n",
        encoding="utf-8")
    log.warning("[auth] 已生成管理令牌（首启/升级），见 %s", data_dir / "FIRST_RUN_TOKEN.txt")
    return token
