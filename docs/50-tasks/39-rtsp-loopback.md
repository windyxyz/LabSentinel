# 任务 39 · 真实摄像头等价演练：go2rtc RTSP 回环 + 断线重连

> 背景：发布唯一硬阻塞是「真实 RTSP 摄像头 24h」。手头没有摄像头，但可以用 go2rtc 把本地演示视频发布成 RTSP，等价验证服务端的 RTSP 接入、分析、事件与**断线重连**逻辑。这是 M1.5 出口条件里机器能自动验证的部分。

## 目标

1. **起 RTSP 模拟源**（只用 1984/8554，不碰 8000/5173）：
   - 写临时配置 `logs/rtsp-sim.yaml`：
     ```yaml
     api: { listen: ":1984" }
     rtsp: { listen: ":8554" }
     streams:
       lab: "ffmpeg:ffmpeg -re -stream_loop -1 -i \"D:/User/Desktop/TOOLS/监控识别/m0-spike/data/lab-walk-24076.mp4\""
     ```
   - 用 `tools/go2rtc.exe --config logs/rtsp-sim.yaml` 启动（`tools/ffmpeg` 加入 PATH）。
   - 用 `tools/ffmpeg/ffprobe.exe -rtsp_transport tcp rtsp://127.0.0.1:8554/lab` 确认能读到视频流（打印编码/分辨率）。
2. **服务端按 RTSP 接入**（8001 + 临时数据目录，避免污染主库）：
   - 环境变量：`LS_DB_PATH=server/data/rtsp_test.db`、`LS_SNAPSHOT_DIR=server/data/rtsp_snapshots`、`LS_VIDEO_DIR=server/data/rtsp_videos`、`LS_ANNOTATION_POOL_DIR=server/data/rtsp_pool`；起 `uvicorn app.main:app --port 8001`。
   - API 创建摄像头 `type=rtsp`、`source=rtsp://127.0.0.1:8554/lab`、`autostart=false`；画安全区、加 `enter`/`enter_no_ppe` 规则；启动。
   - 轮询事件（最长 60s），确认：camera status=running、fps>0、至少 1 条 enter 事件、截图可访问（200）。
3. **断线重连**：
   - `taskkill` 掉 go2rtc → 等 5s → 重新启动同一配置；
   - 观察服务端日志/接口：worker 不应死亡（非文件源读失败会每秒重连）；fps 恢复、状态回到 running、继续出事件（可用事件 id 增长判断）。
   - 记录从 go2rtc 恢复到重新出事件的时间。
4. **清理**：停摄像头、删除测试摄像头/区域/规则（用 API），停 8001 与 go2rtc；删除临时 db/截图/视频目录与 rtsp-sim.yaml；确认 1984/8554/8001 无监听。
5. **报告**：`docs/54-rtsp-loopback-report.md`——配置、命令、时间线、事件/截图证据、重连耗时、发现的问题与建议。

## 约束

- 只用 8001/1984/8554；不碰 8000/5173；不训练；不做其它推理并发（这是一个重任务，跑完再干别的）。
- 测试数据全部写临时目录并在结束时删除；不要用主库 `server/data/labsentinel.db`。
- 不改业务代码；如发现重连/RTSP 处理 bug，写进报告"建议任务"，不要顺手改。
- go2rtc 的 ffmpeg 源要循环播放（`-stream_loop -1`），否则视频播完流就断，无法区分"源结束"与"断线"。

## 验收标准

- ffprobe 能读到 `rtsp://127.0.0.1:8554/lab`（h264，分辨率正确）；
- 服务端 RTSP 摄像头 status=running、fps>0、产生事件且截图 200；
- 杀掉并重启 go2rtc 后，worker 自动恢复（fps>0 + 新事件），报告给出恢复耗时；
- 清理完成、端口空闲、主库未被写入；
- 报告含完整证据与建议。

## 执行报告

### 执行报告（flash，2026-10-28 04:55 交付；完整证据见 [docs/54-rtsp-loopback-report.md](../../54-rtsp-loopback-report.md)）

- **RTSP 接入**：go2rtc 1.9.15 本构建不支持 `ffmpeg:` 源，改用 `exec:ffmpeg … -f mpegts pipe:1`；
  ffprobe 确认 h264/1280×720；服务端 8001 临时库接入后 status=running、**fps=25**、**6s 出首条 enter**、
  60s 累计 20+ 事件、截图 200。
- **断线重连**：`taskkill go2rtc` 后 worker 不死亡、status 保持 running；重启 go2rtc 后 **≈2s 恢复**
  （fps 24.9 + 新事件），无需人工干预。
- **发现**：R1 建议统一用 `exec:` 做文件→RTSP 常驻模拟源；R2 fps 统计窗口导致断线时读数缓慢回落
  （非缺陷，M2 可讨论显式 offline 状态）。
- **清理**：rtsp_test.db / rtsp_data / rtsp-sim.yaml 全删，1984/8554/8001 无监听，主库零写入。
- **主线验收（2026-10-28 07:22）**：报告与清理证据齐全；pytest 84 passed；RTSP/重连结论可用。
