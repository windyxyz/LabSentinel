"""单元测试：任务126 插件一期（清单校验/状态持久化/fake Popen/token 注入/日志安全）。
运行: pytest tests/test_plugins.py -q"""
import json
import subprocess
import sys
import types

import pytest

from app import plugins as pm
from app.config import settings
from app.plugins import PluginManager, PluginManifest


GOOD = {"id": "event-logger", "name": "事件日志外挂", "version": "0.1.0",
        "description": "示例", "command": ["{python}", "main.py"],
        "permissions": ["read"], "autostart": False, "env": {"K": "v"}}


def _mk_plugin(root, pid="event-logger", manifest=None, entry=True):
    d = root / pid
    d.mkdir(parents=True, exist_ok=True)
    (d / "plugin.json").write_text(json.dumps(manifest or GOOD), encoding="utf-8")
    (d / "main.py").write_text("print('plugin')", encoding="utf-8")
    if entry:
        (d / "logs").mkdir(exist_ok=True)
        (d / "logs" / "plugin.log").write_text("line1\nline2\nline3", encoding="utf-8")
    return d


@pytest.fixture()
def mgr(tmp_path, monkeypatch):
    monkeypatch.setattr(settings, "admin_token", "")
    monkeypatch.setattr(settings, "read_token", "")
    monkeypatch.setattr(settings, "auth_read_protected", False)
    return PluginManager(tmp_path / "plugins")


# ---------- 清单校验 ----------

def test_manifest_valid():
    mf = PluginManifest.model_validate(GOOD)
    assert mf.id == "event-logger" and mf.permissions == ["read"]


def test_manifest_rejects():
    bad = [
        dict(GOOD, id="Bad_ID"),                                        # id 规范
        dict(GOOD, id="a"),                                             # 太短
        dict(GOOD, command="python main.py"),                           # shell 字符串
        dict(GOOD, command=[]),                                         # 空
        dict(GOOD, permissions=["read", "admin"]),                      # 越界权限
        dict(GOOD, env={"LS_ADMIN_TOKEN": "x"}),                        # 劫持 LS_*
        dict(GOOD, extra_field=1),                                      # extra forbid
        dict(GOOD, command=[123]),                                      # 非字符串
    ]
    for b in bad:
        with pytest.raises(Exception):
            PluginManifest.model_validate(b)


# ---------- 扫描 ----------

def test_scan_and_bad_cases(tmp_path, mgr):
    _mk_plugin(tmp_path / "plugins")
    (tmp_path / "plugins" / "broken").mkdir()
    (tmp_path / "plugins" / "broken" / "plugin.json").write_text("{bad", encoding="utf-8")
    (tmp_path / "plugins" / "stray.txt").write_text("x", encoding="utf-8")  # 非目录忽略
    items = {x["id"]: x for x in mgr.scan()}
    assert items["event-logger"]["name"] == "事件日志外挂"
    assert "清单非法" in items["broken"]["last_error"]
    assert "stray.txt" not in items
    assert items["event-logger"]["running"] is False


def test_scan_missing_dir(tmp_path):
    m = PluginManager(tmp_path / "nonexistent" / "plugins")
    assert m.scan() == []


# ---------- enable / disable 持久化 ----------

def test_enable_disable_persist(tmp_path):
    _mk_plugin(tmp_path / "plugins")
    m1 = PluginManager(tmp_path / "plugins")
    assert m1.enable("event-logger") is True
    assert m1.enable("nope") is False
    m2 = PluginManager(tmp_path / "plugins")  # 新实例读回状态
    assert "event-logger" in m2._enabled
    m2.disable("event-logger")
    m3 = PluginManager(tmp_path / "plugins")
    assert "event-logger" not in m3._enabled


def test_corrupted_state_falls_back_disabled(tmp_path):
    d = tmp_path / "plugins"
    _mk_plugin(d)
    (d / "state.json").write_text("{corrupted", encoding="utf-8")
    m = PluginManager(d)
    assert m._enabled == set()  # 回退全禁用不崩


def test_disabled_cannot_start(mgr, tmp_path):
    _mk_plugin(mgr.dir)
    ok, msg = mgr.start("event-logger")
    assert ok is False and "未启用" in msg


# ---------- start：fake Popen（不执行真实代码） ----------

class FakeProc:
    def __init__(self):
        self.pid = 4242
        self.returncode = None

    def poll(self):
        return self.returncode

    def terminate(self):
        self.returncode = 0

    def kill(self):
        self.returncode = -9

    def wait(self, timeout=None):
        return self.returncode


def test_start_env_and_cwd(mgr, monkeypatch):
    d = _mk_plugin(mgr.dir)
    captured = {}

    def fake_popen(argv, cwd=None, env=None, stdout=None, stderr=None, creationflags=0):
        captured.update(argv=argv, cwd=cwd, env=env, flags=creationflags)
        return FakeProc()

    monkeypatch.setattr(pm.subprocess, "Popen", fake_popen)
    mgr.enable("event-logger")
    monkeypatch.setattr(settings, "admin_token", "")
    ok, msg = mgr.start("event-logger")
    assert ok is True and "4242" in msg
    assert captured["cwd"] == str(d)
    assert "LS_PLUGIN_ID" in captured["env"] and captured["env"]["LS_PLUGIN_ID"] == "event-logger"
    assert "LS_PLUGIN_DATA" in captured["env"]
    assert "LS_ADMIN_TOKEN" not in captured["env"]  # 防继承
    entry = mgr.scan()[0]
    assert entry["running"] is True and entry["pid"] == 4242


def test_start_entry_outside_dir_rejected(mgr):
    bad = dict(GOOD, command=["D:/evil/run.exe"])
    _mk_plugin(mgr.dir, manifest=bad)
    mgr.enable("event-logger")
    ok, msg = mgr.start("event-logger")
    assert ok is False and "插件目录内" in msg


def test_start_entry_sibling_prefix_rejected(mgr):
    """回归：`../event-logger-evil` 这类同前缀兄弟目录不得绕过目录校验。"""
    bad = dict(GOOD, command=["../event-logger-evil/run.py"])
    _mk_plugin(mgr.dir, manifest=bad)
    evil = mgr.dir / "event-logger-evil"
    evil.mkdir()
    (evil / "run.py").write_text("print('evil')", encoding="utf-8")
    mgr.enable("event-logger")
    ok, msg = mgr.start("event-logger")
    assert ok is False and "插件目录内" in msg


def test_start_python_placeholder(mgr, monkeypatch):
    d = _mk_plugin(mgr.dir, manifest=dict(GOOD, command=["{python}", "main.py"]))
    captured = {}

    def fake_popen(argv, cwd=None, env=None, **kw):
        captured.update(argv=argv, cwd=cwd)
        return FakeProc()

    monkeypatch.setattr(pm.subprocess, "Popen", fake_popen)
    mgr.enable("event-logger")
    ok, _ = mgr.start("event-logger")
    assert ok is True
    assert captured["argv"][0] == sys.executable
    assert captured["argv"][1] == "main.py"
    assert captured["cwd"] == str(d)


def test_token_injection_rules(mgr, monkeypatch):
    d = _mk_plugin(mgr.dir)
    captures = []
    monkeypatch.setattr(pm.subprocess, "Popen",
                        lambda argv, cwd=None, env=None, **kw:
                        captures.append(env) or FakeProc())
    # read 权限 + 读保护未开 → 不注入
    mgr.enable("event-logger")
    mgr.start("event-logger")
    assert "LS_PLUGIN_TOKEN" not in captures[-1]
    mgr.stop("event-logger")  # FakeProc 永活：不停则下次 start 早退不换 env
    # read 权限 + 读保护 + read_token → 注入 read_token
    monkeypatch.setattr(settings, "auth_read_protected", True)
    monkeypatch.setattr(settings, "read_token", "ro-token")
    mgr.start("event-logger")
    assert captures[-1]["LS_PLUGIN_TOKEN"] == "ro-token"
    mgr.stop("event-logger")
    # write 权限 + admin_token → 注入 admin_token
    write_mf = dict(GOOD, permissions=["read", "write"])
    _mk_plugin(mgr.dir, manifest=write_mf)
    mgr.enable("event-logger")
    monkeypatch.setattr(settings, "admin_token", "adm-token")
    mgr.start("event-logger")
    assert captures[-1]["LS_PLUGIN_TOKEN"] == "adm-token"
    assert "adm-token" not in json.dumps(mgr.scan())  # token 绝不进清单/状态输出


def test_stop_grace_then_kill(mgr, monkeypatch):
    _mk_plugin(mgr.dir)
    calls = []

    class SlowProc(FakeProc):
        def __init__(self):
            super().__init__()
            self.killed = False

        def terminate(self):
            calls.append("terminate")  # 拒绝优雅退出（不设 returncode）

        def poll(self):
            return self.returncode if self.killed else None

        def kill(self):
            calls.append("kill")
            self.killed = True
            self.returncode = -9

        def wait(self, timeout=None):
            calls.append(f"wait{timeout}")
            if not self.killed:
                raise subprocess.TimeoutExpired("x", timeout)
            return self.returncode

    monkeypatch.setattr(pm.subprocess, "Popen", lambda *a, **k: SlowProc())
    mgr.enable("event-logger")
    mgr.start("event-logger")
    ok, msg = mgr.stop("event-logger")
    assert ok is True
    assert calls[0] == "terminate" and "kill" in calls  # 先宽限后强杀


# ---------- 日志 ----------

def test_logs_tail_and_path_safety(mgr):
    _mk_plugin(mgr.dir)
    assert mgr.logs("event-logger", tail=2) == "line2\nline3"
    assert mgr.logs("event-logger", tail=100) == "line1\nline2\nline3"
    assert mgr.logs("../evil", tail=2) == ""  # id 正则拒绝
    assert mgr.logs("nonexistent", tail=2) == ""


# ---------- 进程退出后状态 ----------

def test_exit_status_visible(mgr, monkeypatch):
    _mk_plugin(mgr.dir)
    proc = FakeProc()
    monkeypatch.setattr(pm.subprocess, "Popen", lambda *a, **kw: proc)
    mgr.enable("event-logger")
    mgr.start("event-logger")
    assert mgr.scan()[0]["running"] is True
    proc.returncode = 3  # 模拟退出
    entry = mgr.scan()[0]
    assert entry["running"] is False and entry["returncode"] == 3


# ---------- API ----------

@pytest.fixture()
def client(tmp_path, monkeypatch):
    from fastapi.testclient import TestClient
    from app.main import app
    monkeypatch.setattr(pm, "manager", PluginManager(tmp_path / "plugins"))
    with TestClient(app) as tc:
        yield tc, tmp_path / "plugins"


def test_api_flow_and_401(client, monkeypatch):
    tc, pdir = client
    _mk_plugin(pm.manager.dir)  # lifespan 会重建 manager（settings.data_dir/plugins）——写实际目录
    monkeypatch.setattr(settings, "admin_token", "sec")  # 先启用鉴权
    assert tc.get("/api/plugins").status_code == 401  # 无令牌拒
    h = {"X-Admin-Token": "sec"}
    r = tc.get("/api/plugins", headers=h)
    assert r.status_code == 200 and r.json()["plugins"][0]["id"] == "event-logger"
    assert tc.post("/api/plugins/event-logger/start", headers=h).status_code == 400  # 未启用
    assert tc.post("/api/plugins/event-logger/enable", headers=h).json()["enabled"] is True
    r2 = tc.post("/api/plugins/event-logger/start", headers=h)
    # fake 环境未注入 Popen mock → 真启动 python（main.py 打印即退）——只要不 500
    assert r2.status_code in (200, 400)
    assert tc.get("/api/plugins/event-logger/logs?tail=5", headers=h).json()["id"] == "event-logger"
    assert tc.post("/api/plugins/event-logger/disable", headers=h).json()["enabled"] is False
    assert tc.post("/api/plugins/nope/enable", headers=h).status_code == 404
    monkeypatch.setattr(settings, "admin_token", "")
