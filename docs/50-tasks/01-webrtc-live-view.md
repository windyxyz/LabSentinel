# 任务 01 · 实时画面（go2rtc/WebRTC 集成）【主力模型任务】

> 先读本目录 README.md。此任务涉及外部组件与流媒体协议，建议主力模型执行；flash 模型可做其中的 Web 播放器部分（见文末拆分）。

## 背景

Web 端目前只能看快照（`GET /api/cameras/{id}/snapshot`，1Hz 手动刷新）。需求：实时画面延迟 ≤2 秒。M0 预研结论（docs/20-research/00-synthesis.md）：**go2rtc** 作为流媒体网关，RTSP → WebRTC 输出，浏览器用标准 WebRTC API 播放。

## 目标架构

```
RTSP 摄像头 ──┬── go2rtc(端口 1984) ── WebRTC ── 浏览器 <video>
              └── server worker（分析，现状不变）
```

关键点：go2rtc 与分析 worker **共用同一路 RTSP 源**（go2rtc 会做流复制，不会让摄像头出双份流量；若无摄像头可用文件源演示：go2rtc 支持 `ffmpeg:` 源读文件）。

## 实施步骤

1. **部署集成**：下载 go2rtc Windows amd64 单文件（GitHub release，走 ghfast 镜像，放 `tools/go2rtc.exe`，写 `tools/go2rtc.yaml`）；提供 `scripts/start-all.bat`（启动 go2rtc + uvicorn + web dev）
2. **服务端**：新增 `GET /api/cameras/{id}/webrtc` 返回该路流的 go2rtc WebRTC websocket 地址（`ws://<host>:1984/api/ws?src=<camera_id>`）；go2rtc 配置由服务端根据已登记摄像头**自动生成**（改摄像头/启动/停止时重写 yaml 并提示重载——go2rtc 支持 API 热加载 `/api/reload`，研究其文档后选择实现，写进执行报告）
3. **Web 端**：`views/Cameras.vue` 每路加"实时画面"按钮 → 弹层内 WebRTC 播放器（手写 RTCPeerClient：createOffer → go2rtc ws 信令 → ontrack → video.srcObject；不引入 webrtc-streamer/WHIP 库）；断流自动重连（指数退避）
4. **文档**：README 增补 go2rtc 说明与防火墙端口提示

## 已知坑

- go2rtc WebRTC 用 ICE，局域网需 host 候选可达；浏览器 mDNS 候选可能失败——测试时用 `http://localhost` 或明确 IP
- 视频 codec：多数 RTSP 是 H.264 可直接转发；HEVC 需浏览器支持（M0 未验，遇到则 go2rtc 配转码）
- 不要用 HLS 替代（延迟 5s+ 不达标）

## 验收标准

1. 有 RTSP 源时端到端延迟 ≤2s（手机"IP 摄像头"App 模拟，对照秒表录屏）
2. 无摄像头时文件源可演示（循环播放）
3. pytest 全绿、npm build 通过；快照功能不回归
4. 执行报告写清：go2rtc 版本、配置样例、延迟实测数

## flash 子任务拆分（可单独发）

仅第 3 步 Web 播放器组件：给定 ws 信令地址与 <video> 元素，实现 `web/src/components/WebrtcPlayer.vue`（props: src/wsUrl；含重连；模拟 ws 测试），不依赖真实 go2rtc。

## 执行报告（播放器组件部分）

> 执行时间：2026-10-25；范围：仅上节「flash 子任务拆分」的播放器组件，未改任何现有业务文件（`web/src/views/`、`App.vue`、`api.js`、`server/` 均未动）。

### 新建文件清单

| 文件 | 说明 |
|---|---|
| `web/src/components/WebrtcPlayer.vue` | go2rtc WebRTC 播放器组件（零第三方库，手写 RTCPeerConnection 信令）。用法示例写在文件顶部注释块，含完整 wsUrl 示例 |
| `web/mock/go2rtc-mock.mjs` | 最小 go2rtc 信令 mock（Node，零业务依赖）。启动：`node web/mock/go2rtc-mock.mjs [--port 1984] [--close-after N]`；内置自测：`--selftest` |
| `web/package.json` / `package-lock.json` | 仅新增 devDependency `ws@^8.22.0`（mock 的 WebSocket 服务端实现所需；**协议 MIT**，按 AGPL-3.0 项目规则在此报备，仅 devDependency 不进产物） |

组件 props：`wsUrl`（必填，完整信令地址，如 `ws://localhost:1984/api/ws?src=1`）、`label`（可选，左上角叠加名称）。视频元素 `muted autoplay playsinline`、`object-fit: contain` 铺满容器、纯黑底；状态角标叠左下（连接中/已连接/重连中(第N次)/已断开），配色复用 `style.css` 的 CSS 变量（绿色为播放器内新增）。

### 信令协议实现要点（go2rtc 官方 demo 同款）

1. 每次 `connect()` 全新建 `RTCPeerConnection`（旧实例状态污染防护），`addTransceiver('video'/'audio', {direction:'recvonly'})` → `createOffer` → `setLocalDescription` → 发 `{type:'webrtc', value: btoa(localDescription.sdp)}`
2. 收 `{type:'webrtc', value}` → `setRemoteDescription({type:'answer', sdp: atob(value)})`；解析失败视为握手失败，立即走退避重连
3. `pc.onicecandidate` → 发 `{type:'webrtc/candidate', value: btoa(candidate)}`；candidate 为 null（收集完成）时 value 发空字符串照发
4. 收 `{type:'webrtc/candidate', value}` → `addIceCandidate({candidate: atob(value)})`，失败静默（候选不可达属正常）
5. `ontrack` → `video.srcObject = event.streams[0]`（无 streams 时兜底 `new MediaStream([track])`）→ `play()`，同时退避计数清零、状态转「已连接」
6. 重连触发源共 4 个：ws `onclose`、ICE `failed/disconnected` 持续 3s 未恢复（观察窗防抖）、`RTCPeerConnection.connectionState failed/disconnected` 兜底、发 offer 后 10s 未收到 answer 超时。退避 `min(500×2^(n-1), 15000)` ms，`ontrack` 成功归零
7. 组件卸载：`stopped` 标志 + 清空全部定时器 + 摘除 ws/pc 全部回调后再 close，杜绝卸载后回调复活

### mock 验证输出摘要（`node web/mock/go2rtc-mock.mjs --selftest`）

自测客户端 1:1 复刻组件的信令序列与退避参数（Node 侧无 RTCPeerConnection，offer 用同构假 SDP），服务端回显 offer 的 m 行/mid 生成格式合法的假 answer 并附一个假服务器候选。7 轮循环验证，关键输出：

- 信令握手（offer 发出 → 假 answer 收到，共 7 轮）：通过；answer SDP 格式 9 项逐项校验（v=0/o/s/t、2 个 m 行、mid 回显、sendonly、ice-ufrag/pwd、sha-256 指纹、setup:passive、rtcp-mux）：全部通过
- 服务端假候选接收（addIceCandidate 路径）：通过
- 断开 → 下次连接退避间隔（实测 vs 理论）：

  ```
  轮次   实测间隔   理论退避
    2      510ms     500ms
    3     1014ms    1000ms
    4     2011ms    2000ms
    5      502ms     500ms   ← 第 4 轮 ontrack 重置生效，回到起步值
    6     1011ms    1000ms
    7     2013ms    2000ms
  ```
- 理论退避序列（连续失败 1..8 次，ms）：`500, 1000, 2000, 4000, 8000, 15000, 15000, 15000`（第 6 次起被 15s 上限封顶）
- 另对标准模式（默认端口 1984）冒烟：HTTP 健康页正常，真实地址 `ws://localhost:1984/api/ws?src=1` 完成一次 offer → answer + 候选往返，耗时 22ms

自测还抓出并修复了一个真 bug：answer 的 mid 行收集 `slice(5)` 应为 `slice('a=mid:'.length)`，否则生成 `a=mid::0`，真实浏览器 `setRemoteDescription` 会因 mid 不匹配失败。

### npm build 结果

`cd web && npm run build` 通过（`✓ built in 902ms`，16 modules transformed，产物与改动前体积一致——组件暂无引用方，属预期；接线由 Cameras.vue 主任务完成）。组件 SFC 另用项目内 `vue/compiler-sfc` 单独校验：script setup / template / scoped style 三段编译均通过。

### 遗留问题

1. 组件与 mock 的退避常量（500/15000/观察窗 3s/answer 超时 10s）目前两处各自定义，已注释互相锚定；若后续抽公共模块可消除重复
2. mock 的假 answer 能通过浏览器 `setRemoteDescription` 解析但 ICE 永远不通，真实浏览器联调时组件会按「ICE failed → 退避重连」循环——这是预期的离线行为，验证端到端画面仍需真实 go2rtc（主任务范围）
3. `ws` 仅装在 web devDependencies 且只被 mock（Node 脚本）引用，不进浏览器产物；无协议风险
4. 多路同放（多实例）未实测，组件内无全局状态，理论上可并行；多路并发压测见任务 07

## 执行报告（集成部分）

### 交付物
- `tools/go2rtc.exe`（v1.9.15，GitHub release 直连下载）+ `tools/go2rtc.yaml`（服务端自动生成）
- `tools/ffmpeg/ffmpeg.exe + ffprobe.exe`（BtbN win64 构建，go2rtc 文件源必需）
- `server/app/go2rtc.py`：按已登记摄像头自动生成 yaml + API 热同步；`LS_GO2RTC_API`/`LS_GO2RTC_YAML` 配置
- `server/app/api.py`：新增 `GET /api/cameras/{id}/webrtc` 返回 `ws://<host>:1984/api/ws?src=<id>`
- `web/src/views/Cameras.vue`：每路"实时画面"按钮 → 弹层内 WebrtcPlayer（组件见上方子代理报告）
- `scripts/start-all.bat`：一键启动 go2rtc + uvicorn + web dev（自动给 go2rtc 加 ffmpeg PATH）
- README：go2rtc 说明 + 防火墙端口（8000/5173/1984 TCP+UDP/8554）

### 实测结论（go2rtc 1.9.15）
1. **任务文档写的 `/api/reload` 热加载端点在该版本不存在**（GET/POST 均 404，读源码 `internal/streams/api.go` 确认）。
   改用实测可行的热同步：`PUT /api/streams?name=X&src=Y` 创建/更新流、`DELETE /api/streams?src=X` 删除，
   服务端在摄像头增删/启动时做 diff 同步；go2rtc 未运行时写 yaml 供冷启动。
2. **文件源必须 `ffmpeg:` 前缀**（裸 Windows 路径经 API 校验报 "source not supported"，README 输入模块亦无裸文件项）。
   用文件源演示前需 tools/ffmpeg 就位（start-all.bat 已处理）。RTSP 源原样透传。
3. 端到端实测（隔离 8001 实例 + 隔离 DB）：注册文件源摄像头 → go2rtc `/api/streams` 出现该流（热同步 0 重启）→
   `GET /api/frame.jpeg?src=1` 返回 65KB 正常 JPEG（ffmpeg 源出图验证）→ 删摄像头后流自动移除。
4. `webrtc: listen: ":1984"` 固定 ICE 端口，防火墙只需放行 1984。

### 验收情况
- pytest 全绿（31 passed）、npm build 通过、快照功能未动（回归通过）
- ⏳ **延迟 ≤2s 实测需真实浏览器 + 摄像头**（本环境无法开浏览器）：链路各环节已验证
  （信令协议与 go2rtc 官方 demo 一致、mock 播放器自测通过、真实 go2rtc 出流/出图验证通过），
  建议首路 RTSP 接入时按 roadmap S5 用秒表录屏补测延迟数据
