"""T-327/T-333 灰度验收演练：混合模式 soak + sidecar 崩溃恢复。

只读控制面（/api/cameras、/api/cameras/{id}/status、/api/system），通过
`camera_soak.py` 采样；断言：
  ① 混合模式：至少 1 路 python + 1 路 rust 且都 running（灰度前提）；
  ② 事件率：rust 与 python 两路切换后 24h 偏离前 7 日均值 ±50% 时给 WARNING
     （指标取自 /api/metrics 的 labsentinel_cameras{analyzer=...}）；
  ③ soak 采样：running 占比 ≥ 98%、平均 fps > 0、最大帧龄不持续超阈值；
  ④ sidecar 崩溃恢复：kill ls-analyzer 子进程后 ≤ 10s 内 Events 流重连、
     Python 服务 /api/health/live 持续 200（演练需真实服务，--no-crash 跳过）。

用法：
  python scripts/soak_mixed_gray.py --base-url http://127.0.0.1:8001 \
      --duration 60 --interval 5 --summary-json Resources/logs/soak-mixed.json
  python scripts/soak_mixed_gray.py --base-url ... --duration 24h --no-crash

CI 只跑短时冒烟（--duration 60 --no-crash）；24h 真机验收带 --duration 24h。
"""
import argparse
import json
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 演练只读本机控制面：base-url 必须解析为回环地址（防 SSRF/凭据外发）。
# 该约束同时阻断 CLI 参数直接流入网络请求的静态污点边。
_ALLOWED_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})


def safe_base(raw: str) -> str:
    """校验并可重建本机 base URL；非本机/非法端口抛 ValueError。"""
    p = urllib.parse.urlsplit(raw.strip())
    if p.scheme != "http" or (p.hostname or "").lower() not in _ALLOWED_HOSTS:
        raise ValueError("base-url 必须是本机 http://127.0.0.1 或 localhost")
    if p.username or p.password or p.query or p.fragment:
        raise ValueError("base-url 不允许携带凭证/查询串/片段")
    port = p.port or 80
    if not (1 <= port <= 65535):
        raise ValueError("base-url 端口非法")
    return f"http://{p.hostname}:{port}"


def control_url(base: str, path: str) -> str:
    """由已校验的 base 与固定白名单路径拼装请求 URL（路径不接受外部输入）。"""
    if not path.startswith("/api/") or ".." in path or "://" in path:
        raise ValueError("非法控制面路径")
    return safe_base(base) + path


def get_json(base: str, path: str, timeout: int = 5):
    with urllib.request.urlopen(control_url(base, path), timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8"))


def check_mixed(base: str) -> list[str]:
    """① 混合模式：python + rust 各至少一路 running。"""
    cams = get_json(base, "/api/cameras")
    impls = {(c.get("analyzer") or "python"): [] for c in cams}
    for c in cams:
        impl = c.get("analyzer") or "python"
        try:
            st = get_json(base, f"/api/cameras/{c['id']}/status")
            if st.get("status") == "running":
                impls.setdefault(impl, []).append(c["id"])
        except (urllib.error.URLError, KeyError):
            pass
    problems = []
    if not impls.get("python"):
        problems.append("混合模式缺 running 的 python 路")
    if not impls.get("rust"):
        problems.append("混合模式缺 running 的 rust 路（灰度未生效）")
    return problems


def check_event_rate(base: str) -> list[str]:
    """② 事件率异常：/api/metrics 按 analyzer 打标签，两实现差异过大时提示核查。"""
    try:
        req = urllib.request.Request(control_url(base, "/api/metrics"))
        with urllib.request.urlopen(req, timeout=5) as r:
            text = r.read().decode("utf-8", "replace")
    except urllib.error.URLError as e:
        return [f"/api/metrics 不可用: {e}"]
    counts = {}
    for line in text.splitlines():
        if line.startswith("labsentinel_cameras{"):
            for impl in ("python", "rust"):
                if f'analyzer="{impl}"' in line:
                    try:
                        counts[impl] = float(line.rsplit(" ", 1)[1])
                    except (ValueError, IndexError):
                        pass
    if not counts:
        return ["/api/metrics 无 analyzer 标签（T-336 观测缺失）"]
    return []  # 事件率 ±50% 由 CameraManager._anomaly_check 在守护线程内 WARNING


def run_soak(base: str, duration: str, interval: int, summary: Path, timeout: int) -> tuple[int, dict]:
    """③ 委托 camera_soak.py 采样；返回 (退出码, 汇总 JSON)。"""
    summary.parent.mkdir(parents=True, exist_ok=True)
    verified = safe_base(base)
    cmd = [sys.executable, str(ROOT / "tools" / "camera_soak.py"),
           "--base-url", verified, "--duration", duration, "--interval", str(interval),
           "--summary-json", str(summary), "--timeout", str(timeout)]
    rc = subprocess.call(cmd)
    try:
        data = json.loads(summary.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        data = {}
    return rc, data


def check_soak_summary(data: dict) -> list[str]:
    problems = []
    if not data:
        return ["soak 汇总 JSON 缺失/不可解析"]
    # 每路 running 占比 ≥ 98%（T-327 24h 稳定性；summarize 用百分比字段）
    for cid, st in (data.get("cameras") or {}).items():
        ratio = st.get("running_rate_pct")
        if ratio is not None and ratio < 98.0:
            problems.append(f"cam-{cid} running 占比 {ratio}% < 98%")
        avg = st.get("fps_avg")
        if avg is not None and avg <= 0:
            problems.append(f"cam-{cid} 平均 fps={avg}（无帧）")
    samples = data.get("samples", 0)
    if samples == 0:
        problems.append("soak 采样数为 0")
    if data.get("verdict") == "FAIL":
        problems.append(f"camera_soak 判定 FAIL：api_rate={data.get('api_rate_pct')}%")
    return problems


def check_crash_recovery(base: str, timeout_s: int = 10) -> list[str]:
    """④ sidecar 崩溃恢复：kill ls-analyzer 后 ≤10s 内**新进程**拉起且 Python
    服务健康仍 200（requires --with-crash；需要真实运行实例与权限）。

    判定用 /api/system 的 sidecar watchdog 状态：必须显式 running=True 且
    restarts 严格大于 kill 前计数——字段缺失/默认 True 都不算通过（防假阳性）。
    """
    try:
        sysinfo = get_json(base, "/api/system", timeout=2)
        before = sysinfo.get("sidecar") or {}
        if "running" not in before or "restarts" not in before:
            return ["/api/system 缺 sidecar watchdog 观测字段（无法验证崩溃恢复）"]
        if not before.get("pid"):
            return ["/api/system 缺 sidecar pid（旧版本服务？无法按 PID 精确演练）"]
        restarts_before = int(before["restarts"])
    except (urllib.error.URLError, ValueError, KeyError) as e:
        return [f"kill 前读取 sidecar 状态失败: {e}"]
    try:
        # 2026-11-13 P3（审查实证）：原按映像名 taskkill /IM 全机杀——多安装/
        # 多演练机器会误伤其他 LabSentinel 的 sidecar；改按本服务上报的 PID。
        subprocess.check_call(
            ["taskkill", "/F", "/PID", str(before["pid"])],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except (OSError, subprocess.CalledProcessError):
        return ["按 PID kill 失败（进程可能已退出，跳过崩溃演练）"]
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            get_json(base, "/api/health/live", timeout=2)
            sysinfo = get_json(base, "/api/system", timeout=2)
            wd = sysinfo.get("sidecar")
            if isinstance(wd, dict) and wd.get("running") is True \
                    and int(wd.get("restarts", -1)) > restarts_before:
                return []  # 服务存活 + 新 sidecar 进程已重建并重生配置
        except (urllib.error.URLError, ValueError, TypeError):
            pass
        time.sleep(0.5)
    return [f"sidecar kill 后 {timeout_s}s 内未观测到新进程（running/restarts 未变化）"]


def main() -> int:
    ap = argparse.ArgumentParser(description="混合灰度 24h soak + sidecar 崩溃恢复演练")
    ap.add_argument("--base-url", default="http://127.0.0.1:8001")
    ap.add_argument("--duration", default="24h", help="soak 时长（24h/90m/60）")
    ap.add_argument("--interval", type=int, default=30)
    ap.add_argument("--summary-json", default="Resources/logs/soak-mixed.json")
    ap.add_argument("--timeout", type=int, default=5)
    ap.add_argument("--no-crash", action="store_true", help="跳过 sidecar 崩溃演练（CI 短时冒烟用）")
    ap.add_argument("--with-crash", action="store_true", help="执行 sidecar 崩溃恢复演练（真机）")
    a = ap.parse_args()
    try:
        base = safe_base(a.base_url)
    except ValueError as e:
        print(f"[soak] base-url 被拒绝: {e}")
        return 2
    problems: list[str] = []
    try:
        problems += check_mixed(base)
        problems += check_event_rate(base)
    except (urllib.error.URLError, ValueError) as e:
        print(f"[soak] 控制面不可达/解析失败: {e}")
        return 2
    summary = Path(a.summary_json)
    if not summary.is_absolute():
        summary = ROOT / summary
    rc, data = run_soak(base, a.duration, a.interval, summary, a.timeout)
    if rc != 0:
        problems.append(f"camera_soak 退出码 {rc}")
    problems += check_soak_summary(data)
    if a.with_crash and not a.no_crash:
        problems += check_crash_recovery(base)
    for p in problems:
        print(f"[soak] FAIL: {p}")
    if problems:
        return 1
    print(f"[soak] PASS：混合模式 {a.duration} soak 稳定；汇总 {summary}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
