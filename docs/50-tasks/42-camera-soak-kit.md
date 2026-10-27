# 任务 42 · 摄像头 24h 稳定性验收工具包（soak 脚本 + 清单）

> 背景：发布硬阻塞是「真实 RTSP 摄像头 24h」。用户有硬件但没有统一的记录工具——需要一个脚本持续采样服务端/摄像头状态，产出 CSV 与摘要，作为 24h 验收证据；同时给一份可照做的验收清单。

## 目标

1. **新增 `tools/camera_soak.py`**（Python + httpx，和现有工具风格一致）：
   - 参数：
     ```
     --base-url http://127.0.0.1:8000
     --camera-id N          # 缺省取 /api/system 第一路
     --duration 24h         # 支持 24h / 90m / 3600（秒）
     --interval 30          # 采样间隔秒
     --out logs/soak-<时间戳>.csv
     --timeout 5
     ```
   - 每轮：`GET /api/system`（记录 http 耗时、camera status/fps、today_counts）+ `GET /api/events?limit=1`（最新事件 id）。
   - CSV 列：`ts,api_ok,http_ms,status,fps,events_today,latest_event_id,note`。
   - 运行中检测并累计：API 失败次数、camera 非 running 的采样数/最长持续时间、fps=0 次数、事件 id 停滞时长。
   - 结束时打印摘要：采样数、API 可用率、camera running 占比、最长中断、fps min/avg/max、事件增量、CSV 路径；退出码 0（除非参数错误/起始就连不上）。
   - Ctrl+C 中断也要落盘并打印摘要。
2. **新增 `docs/摄像头24h验收清单.md`**：
   - 前置：真实 RTSP 摄像头、主码流/子码流说明（分析建议子码流）、网络、磁盘、告警渠道（可选）。
   - 步骤：启动服务 → 添加摄像头（type=rtsp）→ 画区/规则 → 启动分析 → 运行 soak 24h → 期间做一次断网/重启摄像头（验证重连）→ 结束后看摘要与日志。
   - 通过标准（建议）：API 可用率 ≥99%、camera running ≥99%、无 >60s 非预期中断、fps 不低于配置分析帧率的 80%、事件持续产生、无整机死机/重启。
   - 证据归档：CSV、`logs/server.log` 摘要、事件截图/视频各 1 份、断线重连记录。
3. **自测**（60 秒，不跑 24h）：
   - 8001 起隔离实例（`LS_FILE_LOOP=true` + 临时 DB/目录），创建文件摄像头并启动；
   - `python tools/camera_soak.py --base-url http://127.0.0.1:8001 --duration 60 --interval 5 --out logs/soak-selftest.csv`
   - 确认 CSV 行数≈12、摘要正常、Ctrl+C 也能落盘；清理临时数据并停 8001。
4. **执行报告**：命令、CSV 前 3 行、摘要输出、发现的问题。

## 约束

- 只占 8001；不碰 8000/5173；不训练；不做其它推理并发。
- 不改服务端业务代码；脚本只读 API。
- 自测用临时数据目录，结束删除；真实 24h 运行由用户执行，脚本不要自动启动。
- 退出码与摘要要便于 CI/人工判断；不要吞掉异常（记录到 note）。

## 验收标准

- `--help` 参数齐全；`24h/90m/3600` 解析正确；
- 60s 自测产出 CSV（≥10 行）与摘要；Ctrl+C 落盘；
- 清单文档含前置/步骤/通过标准/证据归档；
- 清理完成、8001 停止、主库未动；
- 报告含 CSV 样例与摘要。

## 执行报告

（待填：脚本参数 / 自测 CSV 样例 / 摘要 / 清单路径 / 发现与建议 / 清理确认）
