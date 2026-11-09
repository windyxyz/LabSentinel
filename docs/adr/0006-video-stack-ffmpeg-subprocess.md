# 数据面视频栈：ffmpeg 子进程管道（GStreamer 绑定 PoC 缓行）

---
status: accepted
date: 2026-11-07
---

## 背景

重构计划 T-322 原定：在 `ls-ingest/examples/` 写 GStreamer（gstreamer-rs）与
FFmpeg（ffmpeg-next/ffmpeg-sys）两个 PoC，对 4 路 1080p 源测 30 分钟后产出本 ADR。
实际上 L1 数据面直接以 **ffmpeg 子进程 + rawvideo 管道**落地（`engine-rs/crates/
ls-ingest/src/lib.rs`），并已在黄金语料（322/322 逐位对齐）与灰度路径上验证。本文
补记决策与理由，并保留 GStreamer 作为后续硬解升级路径。

## 决定

数据面取流/解码 = **ffmpeg 子进程，stdout 输出 bgr24 rawvideo，Rust 侧按帧读管道**：

```
ffmpeg -rtsp_transport tcp -i <src> [-vf fps=<n>,scale=1280:720] -pix_fmt bgr24 -f rawvideo pipe:1
```

- `analysis_fps > 0`：由 ffmpeg `fps` 滤镜抽帧（省 CPU/管道带宽）；
- `analysis_fps = 0`：**纯解码**（无 -vf），帧序/PTS 不重排——黄金语料对齐的
  关键口径（cv2 逐帧读语义）；
- 断流：管道 EOF + 指数退避重连（`Ingest::backoff`），外部可强杀子进程
  （`ChildSlot`，stop_camera 立即打断阻塞读）。

## Considered Options

- **gstreamer-rs（原定首选，缓行）**：插件化硬解（d3d11/nvdec/vaapi）与 RTSP
  健壮性最强；但绑定面大、Windows 上需要运行时插件集，与"单二进制便携分发"
  目标冲突，且 appsink 帧回调与现有 pipeline 线程模型的集成成本高。留作
  L2/L4 硬解需求明确后（多路 1080p CPU 解码成为瓶颈）的升级路径。
- **ffmpeg-next / ffmpeg-sys（Rust 绑定）**：需要链接 libav*（版本矩阵与
  windows-gnu 工具链匹配困难），构建复杂度与本机无 MSVC/libclang 的现实冲突。
- **ffmpeg 子进程（采用）**：零绑定、跨平台一致、与便携包已分发的 ffmpeg.exe
  复用同一二进制；代价是每帧一次管道拷贝（1280x720x3 ≈ 2.7MB/帧，5fps 下
  ≈13MB/s/路，实测可接受）与无硬解（软解 CPU 占用由 ffmpeg 进程承担，
  不占 Python/Rust 推理线程）。
- **OpenCV rust 绑定 / opencv 子进程方案**：与 Python 侧 cv2 行为对齐困难
  （fps 滤镜重排问题即因此产生），放弃。

## Consequences

- 硬解（D3D11VA/NVDEC）暂缺：CPU 软解上限约 8~12 路 720p@25fps（单机），
  达到 §7.2.5 的 16 路目标需在 L2/L4 引入 gstreamer-rs 或 ffmpeg 硬解参数
  （`-hwaccel d3d11va`），届时以 `ls-ingest/examples/` PoC 补测数据并更新本 ADR。
- 黄金语料口径固定：抽帧/PTS/缩放语义全部由本管道与 `resize_u8_linear_cv2`
  复刻层保证（L1 322/322 逐位一致的基础），改动需重跑对齐测试。
- 逐帧 PTS（VFR 精确时间戳）经 ffprobe 逐帧表获取（`probe_frame_pts`）。
- 存证走 JPEG 帧环 + ffmpeg image2pipe 编码（`ls-analyzer/src/evidence.rs`），
  GOP 级 remux（零重编码）留待硬解升级时一并评估。

## 验证记录

- 黄金语料：`docs/验证记录-S7-L1黄金语料对齐.md`（检测逐位一致 + 轨迹 322/322）。
- 断流重连/文件播完：`ls-ingest` 单测 + sidecar e2e
  （`scripts/verify_sidecar_evidence.py`）。
- USB 拔插与 4 路 1080p 30 分钟 PoC 数据：**未测**（列入真机 24h 验收清单，
  与"真实 RTSP 24h"人工项同批执行）。
