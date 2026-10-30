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

（待填：改动 / 发现结果 / 测试与 build / 遗留）
