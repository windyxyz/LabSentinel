# 性能基线（R2-306 / T-307）

```json
{
  "generated_ts": "2026-11-09 11:37:55",
  "machine": {
    "os": "Windows-11-10.0.26300-SP0",
    "python": "3.12.0",
    "cpu_count": 20,
    "onnxruntime": "1.31.1 providers=['AzureExecutionProvider', 'CPUExecutionProvider']"
  },
  "video": "m0-spike\\data\\lab-walk-24076.mp4",
  "thresholds": {
    "cpu_4cam_fps_per_cam": 4.5,
    "dml_8cam_fps_per_cam": 4.5,
    "rust_vs_python": 1.5,
    "rust_rss_ratio": 0.6
  },
  "rows": [
    {
      "ok": true,
      "implementation": "python-cpu",
      "backend": "cpu",
      "cameras": 1,
      "duration_s": 12.0,
      "frames": 430,
      "fps_total": 35.83,
      "fps_per_camera": 35.83,
      "rss_mb": null
    },
    {
      "ok": true,
      "implementation": "rust-tract",
      "cameras": 1,
      "duration_s": 81.2,
      "note": "offline golden runner; per-camera fps not comparable to online path (2026-11-11 rerun; 2026-11-09 measured 54.9)"
    },
    {
      "ok": true,
      "implementation": "rust-ort",
      "cameras": 1,
      "duration_s": 4.0,
      "note": "offline golden runner; per-camera fps not comparable to online path (2026-11-11 rerun; 2026-11-09 measured 4.4)"
    },
    {
      "ok": false,
      "implementation": "python-dml",
      "reason": "n/a (需 dml 变体环境：requirements-win-dml.lock + DirectX 12 GPU)"
    }
  ],
  "note": "完整 1/4/8 路 × 30 分钟矩阵需在目标硬件上重跑（--duration 1800）。"
}
```

## DML 变体实测（R2-303，2026-11-11 短时冒烟）

- 环境：scratch venv 按 `server/requirements-win-dml.lock` 安装（onnxruntime-directml
  **1.24.4**，providers `['DmlExecutionProvider', 'CPUExecutionProvider']`）；
  GPU = AMD Radeon 880M（Ryzen AI 9 H365 核显，DX12）；`--duration 30 --cameras 1,4`。
- `perf_baseline.py` 的实现标注已改为按 Engine 实际后端输出（`python-<backend>`），
  python-dml 行仅在该环境缺 DML EP 时才出现 n/a——数据不再硬编码。

| 实现 | 路数 | 时长 | 每路 fps | 对照 |
|---|---|---|---|---|
| python-directml | 1 | 30s | **69.43** | CPU 单路 35.83 → **1.94×** |
| python-directml | 4 | 30s | **17.50** | CPU 4 路 9.7 → **1.80×** |

- 4 路总吞吐（69.99 fps）与单路（69.43 fps）几乎持平：30s 内 GPU 远未饱和，
  多路扩展的瓶颈不在 EP；每路 17.5 fps ≫ §14.3 DML 门槛 4.5 fps/路。
- `rss_mb: null`：DML 锁不含 psutil（与 CPU 冒烟同口径）；完整矩阵需补 RSS/CPU%/
  句柄数与事件 P95（后者由 `tools/camera_soak.py` 对运行中服务采样）。
- 结论（符合 Q2-02 预期）：DML 在主流核显上稳定可用，1.0 默认推理可考虑 DML。

## 解读（2026-11-09 短时冒烟，CPU 变体）

- 本轮为 `--duration 12 --cameras 1` 的短时冒烟，用于验证脚本与四种实现的可测性；
  **完整 1/4/8 路 × 30 分钟矩阵尚未在目标硬件上跑完**（`--duration 1800`）。
- Python-CPU：单路约 35.8fps；此前 4 路短测约 9.7fps/路，均高于 §14.3 的 4.5fps 门槛。
- Rust 离线 golden runner 处理同一语料：tract 81.2s、ort 4.0s（2026-11-11 复测；
  11-09 为 54.9s/4.4s，同后端 ORT 明显快于 tract 纯 Rust 解释执行，印证 N-20 的
  判断）。两者是**离线整段视频**耗时，与在线每路 fps 不同口径，不能直接算 1.5×
  吞吐比；该比值需在线多路矩阵实测。
- Python-DML：本机未装 `onnxruntime-directml`（统一锁为 CPU 版），且无 DML EP，
  标为 n/a；需 `requirements-win-dml.lock` + DirectX 12 GPU 环境实测
  （→ 2026-11-11 已补，见上节）。
- psutil 未安装时 `rss_mb` 为 null；完整矩阵需补 RSS/CPU%/句柄数与事件 P95
  （后者由 `tools/camera_soak.py` 对运行中服务采样）。

门槛见 ADR-0009 §性能门槛；未达标前不推荐用户把 analyzer 切到 rust。
