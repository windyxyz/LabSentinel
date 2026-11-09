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
      "duration_s": 54.9,
      "note": "offline golden runner; per-camera fps not comparable to online path"
    },
    {
      "ok": true,
      "implementation": "rust-ort",
      "cameras": 1,
      "duration_s": 4.4,
      "note": "offline golden runner; per-camera fps not comparable to online path"
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

## 解读（2026-11-09 短时冒烟）

- 本轮为 `--duration 12 --cameras 1` 的短时冒烟，用于验证脚本与四种实现的可测性；
  **完整 1/4/8 路 × 30 分钟矩阵尚未在目标硬件上跑完**（`--duration 1800 --cameras 1,4,8`）。
- Python-CPU：单路约 35.8fps；此前 4 路短测约 9.7fps/路，均高于 §14.3 的 4.5fps 门槛。
- Rust 离线 golden runner 处理同一语料：tract 54.9s、ort 4.4s（同后端 ORT 明显快于
  tract 纯 Rust 解释执行，印证 N-20 的判断）。两者是**离线整段视频**耗时，与在线
  每路 fps 不同口径，不能直接算 1.5× 吞吐比；该比值需在线多路矩阵实测。
- Python-DML：本机未装 `onnxruntime-directml`（统一锁为 CPU 版），且无 DML EP，
  标为 n/a；需 `requirements-win-dml.lock` + DirectX 12 GPU 环境实测。
- psutil 未安装时 `rss_mb` 为 null；完整矩阵需补 RSS/CPU%/句柄数与事件 P95
  （后者由 `tools/camera_soak.py` 对运行中服务采样）。

门槛见 ADR-0009 §性能门槛；未达标前不推荐用户把 analyzer 切到 rust。
