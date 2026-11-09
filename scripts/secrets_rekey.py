"""R2-106：把 app_settings 里的 DPAPI 密文从用户范围迁移到机器范围（或反向）。

场景：便携版（安装用户会话）迁移到 WinSW 服务（LocalService 账户）后，
用户范围的 `enc:v1:` 密文在新账户下无法解密，渠道静默失效。

用法（必须在**原加密账户的会话**中执行，否则旧密文解不开）：
    python scripts/secrets_rekey.py --to machine
    python scripts/secrets_rekey.py --to user

只处理 `enc:v1:` 前缀的字段；历史明文与非敏感字段不动。失败逐条报告，
不会半途写坏数据库（每条独立 UPDATE，失败即跳过）。
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "server"))

from app import secretstore  # noqa: E402
from app.config import settings  # noqa: E402


def rekey(to: str) -> int:
    if to not in ("user", "machine"):
        print("--to 只能是 user 或 machine")
        return 2
    old_scope = getattr(settings, "secret_scope", "user")
    settings.secret_scope = old_scope  # 用旧范围解密
    from app.store import connect
    con = connect(str(settings.db_path))
    try:
        rows = con.execute(
            "SELECT key, value FROM app_settings WHERE value LIKE 'enc:v1:%'").fetchall()
    finally:
        con.close()
    if not rows:
        print("没有需要迁移的密文")
        return 0
    print(f"找到 {len(rows)} 条密文，从 {old_scope} 范围解出、以 {to} 范围重新加密")
    ok = 0
    for key, raw in rows:
        plain = secretstore.unprotect(raw, settings.data_dir)
        if not plain:
            print(f"  [跳过] {key}：当前范围无法解密（确认在原账户会话执行）")
            continue
        settings.secret_scope = to
        try:
            new_val = secretstore.protect(plain, settings.data_dir)
        except Exception as e:  # noqa: BLE001
            print(f"  [失败] {key}：重新加密失败 {e}")
            settings.secret_scope = old_scope
            continue
        settings.secret_scope = old_scope
        con = connect(str(settings.db_path))
        try:
            con.execute("UPDATE app_settings SET value=? WHERE key=?", (new_val, key))
            con.commit()
        finally:
            con.close()
        ok += 1
    print(f"完成：{ok}/{len(rows)} 条已迁移；重启服务前把 LS_SECRET_SCOPE 设为 {to}")
    return 0 if ok == len(rows) else 1


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description="DPAPI 密钥范围迁移（R2-106）")
    ap.add_argument("--to", choices=["user", "machine"], required=True)
    a = ap.parse_args()
    sys.exit(rekey(a.to))
