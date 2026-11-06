"""插件一期（任务126）：约定式外挂——`<data_dir>/plugins/<id>/plugin.json` + 任意语言进程。

红线（全部先立住）：
- 插件=代码执行：**默认禁用**，管理员显式 enable；崩溃不自动重启（二期做上限）；
- 权限最小化：permissions ⊆ {read,write}；令牌按权限注入（read→read_token，
  write→admin_token），**绝不写进清单/日志**；
- manifest 严格校验（extra="forbid"）：id 规范、command 必须数组（禁 shell 字符串）、
  env 不允许覆盖 `LS_*` 系统变量、入口必须位于插件目录内；
- 子进程环境：复制 os.environ 后**清除** LS_ADMIN_TOKEN/LS_READ_TOKEN 再注入白名单，
  防意外继承；CREATE_NO_WINDOW（pythonw 场景不闪窗）。
"""
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

from pydantic import BaseModel, ConfigDict, field_validator

from .config import settings

log = logging.getLogger("plugins")

CREATE_NO_WINDOW = 0x08000000
ID_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{1,31}$")
manager: "PluginManager | None" = None  # main.py lifespan 装配

STATE_FILE = "state.json"
LOG_REL = Path("logs") / "plugin.log"
STOP_GRACE_S = 5.0


class PluginManifest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    id: str
    name: str
    version: str
    description: str = ""
    command: list[str]
    permissions: list[str] = []
    autostart: bool = False
    env: dict[str, str] = {}

    @field_validator("id")
    @classmethod
    def _id_ok(cls, v: str) -> str:
        if not ID_RE.match(v):
            raise ValueError("id 必须匹配 ^[a-z0-9][a-z0-9_-]{1,31}$")
        return v

    @field_validator("command")
    @classmethod
    def _command_ok(cls, v: list[str]) -> list[str]:
        if not v or not all(isinstance(x, str) and x.strip() for x in v):
            raise ValueError("command 必须是非空字符串数组（不接受 shell 字符串）")
        return v

    @field_validator("permissions")
    @classmethod
    def _perm_ok(cls, v: list[str]) -> list[str]:
        bad = [p for p in v if p not in ("read", "write")]
        if bad:
            raise ValueError(f"permissions 只允许 read/write，非法: {bad}")
        return v

    @field_validator("env")
    @classmethod
    def _env_ok(cls, v: dict[str, str]) -> dict[str, str]:
        hijack = [k for k in v if k.upper().startswith("LS_")]
        if hijack:
            raise ValueError(f"env 不允许覆盖 LS_* 系统变量: {hijack}")
        return v


class PluginManager:
    def __init__(self, plugins_dir):
        self.dir = Path(plugins_dir)
        self.dir.mkdir(parents=True, exist_ok=True)
        self._lock = threading.Lock()
        self._procs: dict[str, dict] = {}  # id -> {proc, started_at, last_error, returncode}
        self._enabled: set[str] = set()
        self._load_state()

    # ---- 状态持久化（原子写；损坏回退全 disabled）----
    @property
    def _state_path(self) -> Path:
        return self.dir / STATE_FILE

    def _load_state(self) -> None:
        try:
            raw = self._state_path.read_text(encoding="utf-8")
            data = json.loads(raw)
            self._enabled = {str(x) for x in data.get("enabled", [])}
        except FileNotFoundError:
            self._enabled = set()
        except (ValueError, OSError) as e:
            log.warning("插件状态文件损坏，回退全部禁用: %s", e)
            self._enabled = set()

    def _save_state(self) -> None:
        tmp = self._state_path.with_suffix(".tmp")
        tmp.write_text(json.dumps({"enabled": sorted(self._enabled)}, ensure_ascii=False),
                       encoding="utf-8")
        os.replace(tmp, self._state_path)

    # ---- 扫描 ----
    def scan(self) -> list[dict]:
        """只读 data/plugins 下一级目录的 plugin.json；坏清单/坏 JSON 生成 error 项不崩。"""
        out = []
        if not self.dir.exists():
            return out
        for d in sorted(self.dir.iterdir()):
            if not d.is_dir():
                continue
            mf_path = d / "plugin.json"
            entry = {"id": d.name, "dir": str(d), "enabled": d.name in self._enabled,
                     "running": False, "pid": None, "started_at": None,
                     "returncode": None, "last_error": None}
            try:
                mf = PluginManifest.model_validate(json.loads(mf_path.read_text(encoding="utf-8")))
            except FileNotFoundError:
                entry["last_error"] = "缺 plugin.json"
                out.append(entry)
                continue
            except (ValueError, json.JSONDecodeError) as e:
                entry["last_error"] = f"清单非法: {e}"
                out.append(entry)
                continue
            if mf.id != d.name:
                entry["last_error"] = f"清单 id {mf.id} 与目录名 {d.name} 不一致"
                out.append(entry)
                continue
            entry.update(mf.model_dump())
            proc_info = self._procs.get(mf.id)
            if proc_info:
                proc = proc_info["proc"]
                if proc.poll() is None:
                    entry["running"] = True
                    entry["pid"] = proc.pid
                else:
                    entry["returncode"] = proc.returncode  # 已退出：状态可见
            out.append(entry)
        return out

    def _manifest(self, pid: str) -> tuple[Path, PluginManifest] | None:
        d = self.dir / pid
        if not ID_RE.match(pid) or not d.is_dir():
            return None
        try:
            mf = PluginManifest.model_validate(
                json.loads((d / "plugin.json").read_text(encoding="utf-8")))
        except (FileNotFoundError, ValueError, json.JSONDecodeError):
            return None
        return d, mf

    # ---- enable / disable ----
    def enable(self, pid: str) -> bool:
        if self._manifest(pid) is None:
            return False
        with self._lock:
            self._enabled.add(pid)
            self._save_state()
        return True

    def disable(self, pid: str) -> bool:
        if self._manifest(pid) is None:
            return False
        with self._lock:
            self._enabled.discard(pid)
            self._save_state()
        if pid in self._procs and self._procs[pid]["proc"].poll() is None:
            self.stop(pid)  # 禁用运行中的插件：一并停止
        return True

    # ---- token 注入 ----
    def _plugin_token(self, permissions: list[str]) -> str:
        if "write" in permissions and settings.admin_token:
            return settings.admin_token
        if "read" in permissions and settings.auth_read_protected and settings.read_token:
            return settings.read_token
        return ""

    # T-112（S-11）：插件子进程环境白名单——SMTP/GenAI/MQTT/告警 webhook 等机密
    # 不再随 os.environ 全量继承；只放行运行必需的系统变量 + LS_PLUGIN_*/LS_API_BASE
    _ENV_PASS = frozenset({
        "PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT",
        "TEMP", "TMP", "USERPROFILE", "HOME", "LANG", "LC_ALL",
        "LOCALAPPDATA", "APPDATA", "PROGRAMFILES", "SYSTEMDRIVE",
        "PYTHONUTF8", "PYTHONIOENCODING",
    })

    def _build_env(self, d: Path, mf: PluginManifest) -> dict:
        env = {k: v for k, v in os.environ.items()
               if k.upper() in self._ENV_PASS and not k.upper().startswith("LS_")}
        env["LS_PLUGIN_ID"] = mf.id
        env["LS_API_BASE"] = settings.public_base_url or "http://127.0.0.1:8000"
        (d / "data").mkdir(parents=True, exist_ok=True)
        env["LS_PLUGIN_DATA"] = str(d / "data")
        token = self._plugin_token(mf.permissions)
        if token:
            env["LS_PLUGIN_TOKEN"] = token  # 只进子进程环境，绝不落盘/落日志
        env.update(mf.env)
        return env

    # ---- start / stop ----
    def _resolve_command(self, d: Path, command: list[str]) -> tuple[list[str] | None, str]:
        """解析入口：支持 `{python}` 占位符 / PATH 上的解释器 / 插件目录内路径。

        安全：插件目录外的绝对路径一律拒绝；`..` 逃逸必须 resolve 后仍位于插件目录内。
        相对脚本参数（command[1:]）由 cwd=插件目录 解析，不在此做路径约束。
        """
        first = command[0].strip()
        from_path = False
        if first == "{python}":
            exe = Path(sys.executable)
        else:
            cand = Path(first)
            if cand.is_absolute():
                try:
                    exe = cand.resolve()
                except OSError:
                    return None, "入口路径无法解析"
            elif any(sep in first for sep in ("/", "\\")) or first.lower().endswith(".exe"):
                try:
                    exe = (d / cand).resolve()
                except OSError:
                    return None, "入口路径无法解析"
            else:  # 裸命令：优先 PATH（python/pwsh 等），否则视为插件目录内文件
                found = shutil.which(first)
                try:
                    exe = Path(found).resolve() if found else (d / first).resolve()
                except OSError:
                    return None, "入口路径无法解析"
                from_path = bool(found)
        try:
            root = d.resolve()
            in_plugin = exe.is_relative_to(root)
            is_runtime_python = exe == Path(sys.executable).resolve()
            if not in_plugin and not is_runtime_python and not from_path:
                return None, "入口路径必须在插件目录内（解释器可用 {python} 或 PATH 命令）"
        except OSError:
            return None, "入口路径无法解析"
        if not exe.exists():
            return None, f"入口不存在: {exe.name}"
        return [str(exe), *[str(x) for x in command[1:]]], ""

    def start(self, pid: str) -> tuple[bool, str]:
        with self._lock:
            if pid in self._procs and self._procs[pid]["proc"].poll() is None:
                return False, "已在运行"
        found = self._manifest(pid)
        if found is None:
            return False, "插件不存在或清单非法"
        d, mf = found
        if pid not in self._enabled:
            return False, "插件未启用（先 enable）"
        argv, err = self._resolve_command(d, mf.command)
        if argv is None:
            return False, err
        log_path = d / LOG_REL
        log_path.parent.mkdir(parents=True, exist_ok=True)
        try:
            lf = open(log_path, "ab")
            lf.write(f"\n===== start {time.strftime('%Y-%m-%d %H:%M:%S')} =====\n".encode())
            from .platform.process import spawn_hidden  # T-601/X-06：跨平台（POSIX start_new_session）
            proc = spawn_hidden(
                argv, cwd=str(d), env=self._build_env(d, mf),
                stdout=lf, stderr=subprocess.STDOUT)
            lf.close()  # 子进程已继承句柄；父进程不保留，防 fd 泄漏
        except OSError as e:
            try:
                lf.close()
            except Exception:  # noqa: BLE001
                pass
            return False, f"启动失败: {e}"
        with self._lock:
            self._procs[pid] = {"proc": proc, "started_at": time.time(),
                                "last_error": None, "returncode": None}
        return True, f"pid={proc.pid}"

    def stop(self, pid: str) -> tuple[bool, str]:
        info = self._procs.get(pid)
        if info is None or info["proc"].poll() is not None:
            return False, "未在运行"
        proc = info["proc"]
        proc.terminate()
        try:
            proc.wait(timeout=STOP_GRACE_S)
        except subprocess.TimeoutExpired:
            proc.kill()  # 5s 宽限后强杀
            proc.wait(timeout=3)
        info["returncode"] = proc.returncode
        return True, f"已停止 rc={proc.returncode}"

    def stop_all(self) -> None:
        for pid in list(self._procs):
            if self._procs[pid]["proc"].poll() is None:
                try:
                    self.stop(pid)
                except Exception as e:  # noqa: BLE001
                    log.warning("停止插件 %s 失败: %r", pid, e)

    def autostart_enabled(self) -> list[str]:
        """启动时：对已 enable 且 autostart=true 的插件自动拉起。"""
        started = []
        for entry in self.scan():
            if entry.get("enabled") and entry.get("autostart") and not entry.get("running") \
                    and not entry.get("last_error"):
                ok, msg = self.start(entry["id"])
                if ok:
                    started.append(entry["id"])
                else:
                    log.warning("插件 %s 自启失败: %s", entry["id"], msg)
        return started

    # ---- 日志 ----
    def logs(self, pid: str, tail: int = 200) -> str:
        if not ID_RE.match(pid):
            return ""
        log_path = self.dir / pid / LOG_REL
        try:  # id 正则已防穿越；再校验解析后仍在插件目录内
            if self.dir.resolve() not in log_path.resolve().parents:
                return ""
            lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
            return "\n".join(lines[-max(1, min(tail, 2000)):])
        except OSError:
            return ""
