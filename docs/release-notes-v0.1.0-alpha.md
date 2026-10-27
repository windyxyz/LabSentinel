# Release Notes · LabSentinel v0.1.0-alpha

> Draft for GitHub Release (task 28). **Not published yet.**
> Number discipline: every figure is sourced from repository documents (path noted inline). Capabilities without test-verified numbers are listed as "planned/in progress".

**Status:** alpha — core features complete, hardware-side validation pending (see Known limitations).

## Highlights

- **Local-first lab safety monitoring**: connect existing RTSP cameras, detect "entered safety zone without lab coat", count occupancy, alert in real time — all computation stays on-premise.
- **Evidence-graded PPE detection**: alerts only on explicit "no lab coat" detection; low visibility never triggers a false violation.
- **Event triage loop**: acknowledge / false-positive / ignore every alert; false-positive snapshots automatically feed the annotation pool for model iteration.
- **Reliable alerting**: 6 channels (WeCom / DingTalk / Feishu / WeChat via ServerChan & PushPlus / QQ via Qmsg / SMTP) with 3× exponential-backoff retry, dead-letter tracking and one-click re-send.
- **Incident-driven hardening**: a DirectML concurrent-inference defect (full system hang, 3× Kernel-Power 41) was root-caused and fixed with a process-global inference lock + regression test (docs/51-loadtest-report.md §3, docs/80-risk-register.md A1).

## Features

- Camera onboarding: Hikvision/Dahua RTSP templates, video-file sources; polygon safety zones & rules (enter / no-PPE / exit / occupancy) drawn on the web UI.
- Detection: YOLO11n person detection + fine-tuned PPE model (11-class vocabulary, task 06 toolchain).
- Video evidence: ±2 s H.264 clip per event (browser-playable), snapshots with click-to-zoom.
- Live view: WebRTC via go2rtc gateway (~1 s end-to-end, to be verified on real cameras), auto-reconnect player.
- Reports: daily/weekly/hourly trends (native SVG), summary cards, CSV export (UTF-8 BOM) and one-page PDF monthly report.
- Data retention: configurable auto-cleanup with disk-pressure emergency purge (compliance: minimal storage).
- Admin token auth (optional), multi-channel alert status page with health self-test.
- Dual-theme WinUI 3-style web UI; Flutter Android client skeleton (task 08).

## Measured performance (source: docs/40-roadmap.md, M0 exit table)

| Backend | Throughput (yolo11n, 640) |
|---|---|
| CPU | 32–39 FPS |
| AMD iGPU (DirectML) | 64 FPS |
| AMD NPU (XINT8) | 40 FPS |

ByteTrack vs simple IOU tracker on lab footage: 4 tracked events vs 15 duplicated alerts (docs/40-roadmap.md, M1 record).

## Models & data (not distributed with the repo)

- Models and datasets are **not included**; deployers download them per NOTICE (AGPL ecosystem, CC BY 4.0 datasets with attribution).
- PPE model v1 (10-class, single dataset) ships its evaluation baseline in docs/52-model-eval.md; model v2 (11 classes, 11,420 images merged from 5 datasets) is in training — per-class comparison will be appended to the same document.
- Retraining/quantization is reproducible via tools/ (tasks 06, 11).

## Security & compliance

- Face recognition module is **disabled by default**; enabling requires explicit consent recording (400 without `consent=true`), on-site signage, and provides one-click purge of all face data (docs/部署指南.md compliance checklist).
- No video leaves the network; snapshots/videos have configurable retention (default 180 days) with automatic cleanup.
- Alert webhooks are restricted to official API domains (https-only, no redirects) to prevent SSRF; inference calls are serialized to avoid a DirectML driver defect (documented incident + fix).
- License: AGPL-3.0 (see LICENSE); third-party components declared in NOTICE.

## Known limitations

- WebRTC end-to-end latency and touch UX pending validation on real cameras and devices.
- Event videos require an OpenH264 runtime on the server for browser playback (falls back to MPEG-4 Part 2 otherwise; download documented in docs/部署FAQ.md).
- Model domain gap on merged public datasets is being closed via the data flywheel (docs/90-data-flywheel.md); v1 baseline numbers are intentionally published unpolished (docs/52-model-eval.md).
- No CI-run model tests (CI covers pytest/web build/lint only, by design).
- Single-user admin token auth only; multi-user is planned for M3.

## Installation

- Docker: `docker compose up -d` (see docs/部署指南.md §2)
- Windows native: `scripts\start-all.bat` (health check & stop scripts included; see docs/部署指南.md §1)
- Requirements: Python 3.11+, Node 20 (web dev), models downloaded separately per NOTICE.

## Links (placeholders until the repository is published)

- Repository / Quick start: `README.md`
- Deployment guide & FAQ: `docs/部署指南.md`, `docs/部署FAQ.md`
- Architecture & API: `docs/30-architecture.md`, `docs/35-api-reference.md`
- Measured data: `docs/40-roadmap.md`, `docs/51-loadtest-report.md`, `docs/52-model-eval.md`
