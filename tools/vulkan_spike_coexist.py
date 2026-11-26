"""Exercise the LS-086 Rust Vulkan CV candidate and llama.cpp on the same device."""

import argparse
from base64 import b64encode
from contextlib import ExitStack
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import time
from urllib.error import URLError
from urllib.request import ProxyHandler, Request, build_opener

from vulkan_spike_compare import fingerprint


def windows_graphics_events(start_ms, end_ms):
    script = f"""
$from = [DateTimeOffset]::FromUnixTimeMilliseconds({start_ms}).LocalDateTime
$until = [DateTimeOffset]::FromUnixTimeMilliseconds({end_ms}).LocalDateTime
try {{
    $events = @(Get-WinEvent -FilterHashtable @{{LogName='System'; StartTime=$from; EndTime=$until; Id=@(41,4101)}} -ErrorAction Stop)
}} catch {{
    if ($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') {{ throw }}
    $events = @()
}}
ConvertTo-Json -Compress -InputObject @($events | Where-Object {{
    $_.ProviderName -in @('Microsoft-Windows-Kernel-Power','Display')
}} | Select-Object Id,ProviderName,RecordId)
"""
    result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
                            check=True, capture_output=True, text=True, timeout=20,
                            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    return json.loads(result.stdout)


def stop(process):
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def wait(check, process, timeout, label):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and process.poll() is None:
        if check():
            return
        time.sleep(0.1)
    raise RuntimeError(f"{label} unavailable: process exit code {process.poll()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("cv", "ncnn-dll", "ncnn-model", "llama", "model", "projector", "frame", "image", "out"):
        parser.add_argument("--" + flag, type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--adapter", type=int, default=0)
    parser.add_argument("--vlm-device", default="Vulkan0")
    parser.add_argument("--iterations", type=int, default=600)
    parser.add_argument("--soak-seconds", type=int)
    parser.add_argument("--fps-per-lane", type=int, default=5)
    parser.add_argument("--request-interval", type=int, default=120)
    parser.add_argument("--ppe-model", type=Path, help="ncnn PPE model basename, including .ncnn")
    args = parser.parse_args()
    if args.adapter < 0 or args.iterations < 1:
        parser.error("adapter must be nonnegative and iterations must be positive")
    if args.soak_seconds is not None and (
        not 1 <= args.soak_seconds <= 14_400 or not 1 <= args.fps_per_lane <= 30
        or args.request_interval < 1 or args.ppe_model is None
    ):
        parser.error("soak requires 1..=14400 seconds, 1..=30 FPS, a positive request interval and --ppe-model")
    args.out.mkdir()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    http = build_opener(ProxyHandler({}))
    key = secrets.token_urlsafe(32)
    authorization = {"Authorization": f"Bearer {key}"}
    image = b64encode(args.image.read_bytes()).decode("ascii")
    request_body = json.dumps({
        "model": "ls-vulkan-probe", "temperature": 0, "max_tokens": 20, "cache_prompt": False,
        "messages": [{"role": "user", "content": [
            {"type": "image_url", "image_url": {"url": f"data:image/jpeg;base64,{image}"}},
            {"type": "text", "text": "Is a person visible? Answer with yes or no."},
        ]}],
    }).encode()
    cv_json = args.out / "cv.jsonl"
    cv_diagnostics = args.out / "cv.log"
    vlm_diagnostics = args.out / "llama.log"
    request_log = args.out / "requests.jsonl"
    stop_path = args.out / "stop.requested"
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    with ExitStack() as cleanup:
        vlm_log = cleanup.enter_context(vlm_diagnostics.open("wb"))
        vlm = subprocess.Popen([
            str(args.llama), "--host", "127.0.0.1", "--port", str(port), "--device", args.vlm_device,
            "--parallel", "1", "--ctx-size", "4096", "--batch-size", "256", "--ubatch-size", "128",
            "--api-key", key, "--verbose",
            "--alias", "ls-vulkan-probe", "--n-gpu-layers", "99", "--model", str(args.model),
            "--mmproj", str(args.projector),
        ], cwd=args.llama.parent, stdout=vlm_log, stderr=subprocess.STDOUT, creationflags=creationflags)
        cleanup.callback(stop, vlm)

        def ready():
            try:
                with http.open(Request(base + "/health", headers=authorization), timeout=1) as response:
                    return response.status == 200
            except (URLError, TimeoutError):
                return False

        wait(ready, vlm, 45, "loopback VLM")
        cv_out = cleanup.enter_context(cv_json.open("wb"))
        cv_log = cleanup.enter_context(cv_diagnostics.open("wb"))
        env = {**os.environ, "LS_NCNN_DLL": str(args.ncnn_dll), "LS_NCNN_ADAPTER": str(args.adapter),
               "LS_VULKAN_SPIKE_PHASES": "1"}
        pacing = [] if args.soak_seconds is None else [
            "--soak", str(args.fps_per_lane), str(args.soak_seconds), str(args.ppe_model.resolve()),
        ]
        cv = subprocess.Popen([
            str(args.cv), *pacing, "ncnn-fp16-storage", str(args.ncnn_model), str(args.iterations), str(args.frame),
        ], env=env, stdout=cv_out, stderr=cv_log, creationflags=creationflags)
        cleanup.callback(stop, cv)
        (args.out / "processes.json").write_text(json.dumps({
            "controller": os.getpid(), "cv": cv.pid, "vlm": vlm.pid,
            "soak_seconds": args.soak_seconds, "fps_per_lane": args.fps_per_lane,
        }) + "\n", encoding="utf-8")
        wait(lambda: b"LS_VULKAN_SPIKE_READY 4 " in cv_diagnostics.read_bytes(),
             cv, 90, "four-stream CV measurement")
        completions = []
        began = time.monotonic()
        due = began
        planned = 2 if args.soak_seconds is None else (args.soak_seconds + args.request_interval - 1) // args.request_interval
        for _ in range(planned):
            while time.monotonic() < due and cv.poll() is None:
                if stop_path.exists():
                    raise RuntimeError("stop requested")
                time.sleep(min(0.2, max(0, due - time.monotonic())))
            if stop_path.exists():
                raise RuntimeError("stop requested")
            if cv.poll() is not None:
                break
            start = time.time_ns() // 1_000_000
            request = Request(base + "/v1/chat/completions", request_body,
                              {**authorization, "Content-Type": "application/json"})
            with http.open(request, timeout=30) as response:
                body = json.load(response)
            finish = time.time_ns() // 1_000_000
            if not body["choices"][0]["message"]["content"].strip():
                raise RuntimeError("local VLM returned an empty completion")
            row = {"started_at_ms": start, "finished_at_ms": finish,
                   "completion_tokens": body.get("usage", {}).get("completion_tokens")}
            completions.append(row)
            with request_log.open("a", encoding="utf-8") as log:
                log.write(json.dumps(row) + "\n")
            if args.soak_seconds is not None:
                due = max(due + args.request_interval, time.monotonic())
        deadline = began + (args.soak_seconds or 120) + 30
        while cv.poll() is None and time.monotonic() < deadline:
            if stop_path.exists():
                raise RuntimeError("stop requested")
            time.sleep(0.2)
        if cv.poll() != 0:
            raise RuntimeError(f"CV exited {cv.returncode}")
        if vlm.poll() is not None:
            raise RuntimeError(f"VLM exited early: {vlm.returncode}")

    measurements = [json.loads(line) for line in cv_json.read_text(encoding="utf-8").splitlines()]
    expected_streams = [1, 4] if args.soak_seconds is None else [4]
    if [row["streams"] for row in measurements] != expected_streams:
        raise RuntimeError("CV did not finish the requested stream counts")
    concurrent = measurements[-1]
    cv_log = cv_diagnostics.read_text(encoding="utf-8")
    devices = dict(re.findall(r"^\[(\d+) ([^\]]+)\]\s+queueC=", cv_log, re.M))
    device = devices.get(str(args.adapter))
    vlm_log = vlm_diagnostics.read_text(encoding="utf-8", errors="replace")
    if not device or list(devices.values()).count(device) != 1 \
            or f"using device {args.vlm_device} ({device})" not in vlm_log \
            or not re.search(r"offloaded [1-9]\d*/\d+ layers to GPU", vlm_log):
        raise RuntimeError("Vulkan device names or VLM GPU offload were not confirmed")
    overlap_ms = [max(0, min(row["finished_at_ms"], concurrent["finished_at_ms"])
                      - max(row["started_at_ms"], concurrent["started_at_ms"])) for row in completions]
    if not any(overlap_ms):
        raise RuntimeError("no VLM request overlapped the measured four-stream CV run")
    if re.search(r"VK_ERROR_DEVICE_LOST|device lost", vlm_log + cv_log, re.I):
        raise RuntimeError("Vulkan device loss was logged")
    events = windows_graphics_events(concurrent["started_at_ms"], concurrent["finished_at_ms"]) if os.name == "nt" else None
    failures = []
    if events:
        failures.append("Windows logged a display timeout or unexpected restart during the benchmark")
    if args.soak_seconds is not None:
        if events is None:
            failures.append("Windows event evidence is unavailable")
        if concurrent["elapsed_s"] < args.soak_seconds or len(completions) != planned or not all(overlap_ms):
            failures.append("soak duration or matching VLM request count was incomplete")
        if any(lane["frames"] < args.soak_seconds * args.fps_per_lane * 0.9 for lane in concurrent["lanes"]):
            failures.append("one or more CV lanes fell below 90% of the requested rate")
    evidence = {"gpu": device, "cv_runner": fingerprint(args.cv), "ncnn": fingerprint(args.ncnn_dll),
                "vlm_runner": fingerprint(args.llama), "vlm_model": fingerprint(args.model),
                "projector": fingerprint(args.projector), "measurements": measurements,
                "local_vlm_requests": completions, "overlap_ms": overlap_ms,
                "planned_requests": planned, "windows_graphics_events": events, "soak_seconds": args.soak_seconds,
                "passed": not failures, "failures": failures}
    if args.soak_seconds is not None:
        evidence["cv_models"] = [fingerprint(Path(str(base) + suffix))
                                 for base in (args.ncnn_model, args.ppe_model)
                                 for suffix in (".param", ".bin")]
        evidence["input_frame"] = fingerprint(args.frame)
        evidence["vlm_image"] = fingerprint(args.image)
    (args.out / "report.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    if failures:
        raise RuntimeError("; ".join(failures))
    print(f"{device}: four-stream CV {concurrent['fps']:.2f} fps; VLM overlap {sum(overlap_ms)} ms")


if __name__ == "__main__":
    main()
