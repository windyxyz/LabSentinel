"""单元测试：任务109 deploy.py 三处新机坑（Node rename 降级 / 版本号数值排序 / .env 只修模型行）。
运行: pytest tests/test_deploy_fixes.py -q"""
import os
import pathlib
import sys
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "scripts"))

import pytest  # noqa: E402

import deploy  # noqa: E402


# ---------- 坑2：ffmpeg-static 版本号排序（已移除） ----------
# T-005 起 ffmpeg/ffprobe/Node 版本由 scripts/assets.lock.json 钉死（SHA256 校验），
# deploy 不再做"动态 latest + 数值排序"——原 _version_key 函数与对应测试一并移除，
# 锁文件结构与下载卫生由 tests/test_assets_lock.py 覆盖。


# ---------- 坑1：Node 解压目录就位（目标已存在 / 跨盘 rename 失败） ----------

def _mk_node_dir(p: Path) -> None:
    p.mkdir(parents=True)
    (p / "node.exe").write_bytes(b"new-node")
    (p / "npm.cmd").write_text("npm", encoding="ascii")


def test_promote_fresh_rename(tmp_path):
    src = tmp_path / "extracted"
    _mk_node_dir(src)
    dst = tmp_path / "node"
    deploy._promote_extracted_dir(src, dst)
    assert (dst / "node.exe").read_bytes() == b"new-node"
    assert not src.exists()  # rename 成功，源目录不再存在


def test_promote_target_exists_replaces_stale(tmp_path):
    src = tmp_path / "extracted"
    _mk_node_dir(src)
    dst = tmp_path / "node"
    dst.mkdir()
    (dst / "stale.txt").write_text("old", encoding="ascii")  # 上次半失败残留
    deploy._promote_extracted_dir(src, dst)
    assert (dst / "node.exe").read_bytes() == b"new-node"
    assert not (dst / "stale.txt").exists()  # 旧目录被清掉，不是半新半旧
    assert not src.exists()


def test_promote_rename_failure_falls_back_to_copy(tmp_path, monkeypatch):
    src = tmp_path / "extracted"
    _mk_node_dir(src)
    dst = tmp_path / "node"

    def broken_rename(self, target):  # 模拟跨盘/占用导致 rename 必败
        raise OSError("rename failed")

    monkeypatch.setattr(pathlib.Path, "rename", broken_rename)
    deploy._promote_extracted_dir(src, dst)
    assert (dst / "node.exe").read_bytes() == b"new-node"
    assert not src.exists()  # 复制成功后删源


def test_promote_target_locked_overlays_copy(tmp_path, monkeypatch):
    """旧目录删不掉（rmtree 失败）→ 覆盖复制，新文件必须就位。"""
    src = tmp_path / "extracted"
    _mk_node_dir(src)
    dst = tmp_path / "node"
    dst.mkdir()
    (dst / "stale.txt").write_text("old", encoding="ascii")

    real_rmtree = deploy.shutil.rmtree

    def locked_rmtree(path, *a, **kw):
        if Path(path) == dst:
            raise OSError("access denied")
        return real_rmtree(path, *a, **kw)

    monkeypatch.setattr(deploy.shutil, "rmtree", locked_rmtree)
    deploy._promote_extracted_dir(src, dst)
    assert (dst / "node.exe").read_bytes() == b"new-node"


# ---------- 坑3：ensure_env_file 只修模型行，用户配置逐字保留 ----------

def _set_root(tmp_path, monkeypatch):
    monkeypatch.setattr(deploy, "ROOT", tmp_path)
    model = tmp_path / "m0-spike" / "yolo11n_ppe.onnx"
    model.parent.mkdir(parents=True, exist_ok=True)
    model.write_bytes(b"onnx")
    return model


def test_env_missing_generates_template(tmp_path, monkeypatch):
    _set_root(tmp_path, monkeypatch)
    deploy.ensure_env_file()
    text = (tmp_path / "server" / ".env").read_text(encoding="utf-8")
    assert "LS_PPE_MODEL=" in text
    assert "LS_FILE_LOOP=true" in text


def test_env_valid_untouched(tmp_path, monkeypatch):
    model = _set_root(tmp_path, monkeypatch)
    env = tmp_path / "server" / ".env"
    original = (f"LS_PPE_MODEL={model.as_posix()}\n"
                "LS_WECOM_WEBHOOK=https://qyapi.example/hook\n")
    env.parent.mkdir(parents=True)
    env.write_text(original, encoding="utf-8")
    deploy.ensure_env_file()
    assert env.read_text(encoding="utf-8") == original  # 有效配置逐字不动
    assert list(env.parent.glob(".env.bak-*")) == []  # 也不产生备份


def test_env_stale_model_keeps_user_lines(tmp_path, monkeypatch):
    """模型行失效 → 只替换模型行；注释/告警渠道/其它行逐字保留，且留 .env.bak-*。"""
    model = _set_root(tmp_path, monkeypatch)
    env = tmp_path / "server" / ".env"
    env.parent.mkdir(parents=True)
    env.write_text(
        "# 用户注释\n"
        "LS_PPE_MODEL=D:/other-machine/yolo.onnx\n"
        "LS_WECOM_WEBHOOK=https://qyapi.example/hook\n"
        "LS_FILE_LOOP=true\n",
        encoding="utf-8")
    deploy.ensure_env_file()
    text = env.read_text(encoding="utf-8")
    assert f"LS_PPE_MODEL={model.as_posix()}" in text
    assert "D:/other-machine" not in text
    assert "LS_WECOM_WEBHOOK=https://qyapi.example/hook" in text
    assert "# 用户注释" in text
    assert "LS_FILE_LOOP=true" in text
    baks = list(env.parent.glob(".env.bak-*"))
    assert len(baks) == 1
    assert "D:/other-machine/yolo.onnx" in baks[0].read_text(encoding="utf-8")


def test_env_no_model_line_appends(tmp_path, monkeypatch):
    model = _set_root(tmp_path, monkeypatch)
    env = tmp_path / "server" / ".env"
    env.parent.mkdir(parents=True)
    env.write_text("LS_WECOM_WEBHOOK=keep-me\n", encoding="utf-8")
    deploy.ensure_env_file()
    text = env.read_text(encoding="utf-8")
    assert "LS_WECOM_WEBHOOK=keep-me" in text
    assert f"LS_PPE_MODEL={model.as_posix()}" in text


# ---------- 卸载/重部署时备份文件一并清理 ----------

def test_drop_env_removes_env_backups(tmp_path, monkeypatch):
    _set_root(tmp_path, monkeypatch)
    server = tmp_path / "server"
    server.mkdir()
    (server / ".env").write_text("x=1\n", encoding="utf-8")
    (server / ".env.bak-20261104-000000").write_text("old=1\n", encoding="utf-8")
    monkeypatch.setattr(deploy, "kill_ports", lambda: None)  # 不动真实端口
    monkeypatch.setattr(deploy, "VENV_MARK", tmp_path / "nonexistent" / "venv_path.txt")
    deploy._drop_env()
    assert not (server / ".env").exists()
    assert list(server.glob(".env.bak-*")) == []
