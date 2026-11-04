# 任务 113 · 局域网摄像头发现助手（P1）

> 背景：新用户接入现有摄像头时，不知道 IP/RTSP 路径，只能翻路由器/说明书。本任务做**发现助手**：
> WS-Discovery（ONVIF）发现设备 + 常见品牌 RTSP 模板 + 端口/连通性探测。**不做完整 ONVIF 认证取流**
> （需要账号/媒体服务 SOAP，风险大），只把候选交给用户填账号。

## 目标

### 1. `server/app/discovery.py`

- `ws_discover(timeout=3.0) -> list[dict]`：
  - UDP 组播 `239.255.255.250:3702` 发 ONVIF Probe（NetworkVideoTransmitter），收 XML；
  - 用 `xml.etree.ElementTree` 解析（**禁用外部实体**；只取 `XAddrs`/`Scopes`/`Name`）；
  - 返回 `{ip, xaddr, name, scopes}`，按 ip 去重；超时/无响应返回空；
  - 只允许局域网（结果过滤私有地址：10/8、172.16/12、192.168/16、169.254/16）。
- `probe_host(host, port=554, timeout=1.0) -> dict`：TCP 连接测试 + 可选 `ffprobe`（若存在）
  返回 `{reachable, port_open, ffprobe:{codec,width,height,fps}|None, error}`。
- 并发限制：发现最多 3s、probe 串行 1s；服务端线程池执行，不阻塞事件循环。
- 缓存 30s（避免页面刷新反复组播）。

### 2. API（require_admin，发现涉及内网扫描）

- `GET /api/discovery/cameras?timeout=3&refresh=0` → `{devices:[...],error}`；
- `POST /api/discovery/probe` `{host,port,username,password}`：
  - 只做 TCP/ffprobe 探测；**不把 username/password 写日志/返回**；
  - 若给了凭证且 ffprobe 可用，构造 `rtsp://user:pass@host:554/...` 仅用于本次探测，结果里脱敏。
- 返回 200 语义降级（无设备/网络不可用 error 文案），不 500。

### 3. 前端 `Cameras.vue`

- 「发现摄像头」按钮：列表显示 IP/名称/XAddr/探测结果；每行“填入表单”（source 模板 + 名称）；
- 品牌模板下拉（海康/大华/宇视/通用）生成 `rtsp://用户名:密码@IP:554/...` 占位；
- 提示：需与摄像头同网段；ONVIF 未开启/被防火墙拦截时只能手输；
- 不自动添加/不自动启动；不新增路由。

### 4. 测试 `server/tests/test_discovery.py`

- 解析样例 WS-Discovery XML（1/2 设备、重复、非私有 IP 过滤、畸形 XML 不崩）；
- `probe_host`：mock socket 成功/拒绝/超时；ffprobe 输出解析（h264/h265/失败）；
- API：无设备 200 + error、鉴权 401、refresh 缓存行为；
- 不真的发组播/扫网（测试 mock）。

### 5. 文档

- `35-api-reference`；`部署指南` §3.1 加“发现助手”；`部署FAQ`「搜不到摄像头怎么办」
  （同网段/ONVIF 开启/防火墙/组播/手动 RTSP 模板）。

## 约束

- 只读探测，不修改摄像头配置；不实现 ONVIF 认证/PTZ；
- 不扫描公网/非私有地址；不记录凭证；不引入依赖；
- 前端只改 Cameras.vue/api.js。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 本机实跑：无 ONVIF 设备时返回空 + 提示（不崩）；probe 本机 554 关闭时 error 可读；
- 报告附 API JSON（脱敏）与测试样例。

## 执行报告

**flash 2026-11-04 完成（pytest 431 passed = 416+15 新增；build gzip 58.02KB；本机实跑 PASS）**

### 1. server/app/discovery.py（新文件）

- `ws_discover(timeout=3)`：UDP 组播 `239.255.255.250:3702` 发 ONVIF Probe
  （NetworkVideoTransmitter），收集应答；`_parse_probe_response`（纯函数）只取
  XAddrs/Scopes/name 三字段，**DOCTYPE/ENTITY 一律拒绝**（XXE/实体爆炸加固）；
  结果按 ip 去重、`is_lan` 白名单（10/8、172.16/12、192.168/16、169.254/16——公网/回环全滤）；
  任何异常返回空不抛；
- `get_devices`：30s 缓存（refresh 强制重发），锁外组播；
- `probe_host`：gethostbyname → 白名单校验（公网直接拒）→ TCP create_connection
  （可读 error）→ 可选 ffprobe（找 which → 仓库/包根 tools/ffmpeg，双重根推导）；
  **凭证只进本次探测 URL，返回体/错误串零凭证**（ffprobe stderr 可能回显 URL——不进结果）；
  `eval_fraction`（30000/1001 → 29.97，非法容错 0）；
- `lan_addresses()`（**SPlayer-Next #5 折叠项**）：本机私网 IPv4，192.168 优先、10 段次之、
  169.254 再次、172.x（Hyper-V/WSL 虚拟网卡）殿后——实测本机输出
  `['192.168.1.246', '172.28.231.237', '172.30.207.46', '172.29.176.1']`。

### 2. API（require_admin）

- `GET /api/discovery/cameras?timeout=3&refresh=0` → `{devices, lan, error}`；
  无设备/组播不可用 200+可读提示（不 500）；
- `POST /api/discovery/probe` `{host,port,username?,password?}` → 探测结果（无凭证字段）；
  host 空/port 非法 400；
- 前端「摄像头」页发现面板：品牌模板下拉（海康/大华/宇视/通用）+"搜索/强制刷新"+
  每行"探测 :554"（口开放/编码/分辨率/fps）+"填入表单"（模板占位用户名:密码，
  用户自行改真实账号）；**不自动添加/不启动**；lan 地址提示同网段。

### 3. 测试（test_discovery.py，15 用例，全 mock 不发组播）

is_lan 白名单矩阵、单/双设备 XML 解析、畸形/XXE 拒绝、**公网 IP 过滤**（FakeSock），
probe 开放（带 ffprobe 解析）/拒绝（可读 error）/超时/公网/无法解析、**凭证不回传**
（返回体零凭证、URL 确带凭证为设计）、eval_fraction、API 空+提示/devices+refresh 缓存语义/
probe 校验/鉴权 401。
**过程坑（已修）**：①FakeSock 无 close() + 无限 recvfrom → 测试挂死（改计数后抛 timeout）；
②组合 XML 样例双闭合标签；③eval_fraction("") 抛 ValueError（改容错 0）。

### 4. 本机实跑（8188 临时实例）

- 发现：`devices=0` + "未发现设备：…ONVIF…UDP 3702…"（本机无 ONVIF 设备，不崩）；
- probe 关闭口：`端口 554 连接失败：超时或被拒绝`；probe 公网 8.8.8.8：白名单拒绝 → **PASS**。

### 5. 文档

35-api-reference（两端点+安全边界）、部署指南 §3.1 发现助手、部署FAQ「搜不到摄像头怎么办」。

### 约束遵守

只读探测；不做 ONVIF 认证/PTZ；不扫公网；不记录凭证；零新依赖；前端只改 Cameras.vue/api.js。

### 遗留

- doctor.py 的"访问地址"展示接 `lan_addresses()` 属 0.5h 顺手项，未含在本任务（规格未列）。
