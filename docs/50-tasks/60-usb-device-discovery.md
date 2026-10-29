# 任务 60 · USB 摄像头枚举 API + 前端选择器

> 背景：接 USB 摄像头时用户要自己猜设备索引（`0`/`1`/`2`），新机器上极易选错（本机实测：
> 索引 0 = 物理 HD Webcam，索引 1 = OBS Virtual Camera）。本机 `tools/ffmpeg/ffmpeg.exe` 已能枚举
> DirectShow 设备，把它接进 API 和"添加摄像头"表单即可消除猜索引。

## 目标

### 1. 服务端 `GET /api/cameras/usb-devices`

- 读保护口径与其它 GET 一致：默认公开；`LS_AUTH_READ_PROTECTED=true` 时要求 `X-Admin-Token`
  （复用 `require_read`）。
- 运行：
  ```
  tools/ffmpeg/ffmpeg.exe -hide_banner -f dshow -list_devices true -i dummy
  ```
  `timeout=10`；**该命令必然以非 0 退出**（dummy 输入不存在），只看 stderr。
- 解析 stderr 中形如 `[dshow @ ...] "HD Webcam" (video)` 的行，按出现顺序赋 `index` 0..n-1；
  忽略 `(audio)` 设备。返回：
  ```json
  {"devices":[{"index":0,"name":"HD Webcam","source":"0"}],"error":null,"ffmpeg":"...\\tools\\ffmpeg\\ffmpeg.exe"}
  ```
- 失败降级：ffmpeg 不存在 / 超时 / 无匹配 → **HTTP 200**，`devices=[]`，`error` 给可读中文原因
  （不要 500，前端要能展示"未发现设备，可手动输入索引"）。
- **10 秒内存缓存**（`?refresh=1` 强制刷新），避免每次打开页面都起进程；缓存键固定（本机设备）。
- 代码注释写明限制：索引是 DirectShow 枚举顺序，通常与 OpenCV `CAP_DSHOW` 一致，但驱动/系统更新后
  可能变化；**设备名更稳定，索引仅本机参考**。

### 2. 前端 `web/src/views/Cameras.vue`

- `form.type === 'usb'` 时：
  - `source` 输入框换成 `<select>`（显示 `名称（索引 N）`，值 = index 字符串）+「刷新设备」按钮；
  - 进入页面或切到 USB 时自动拉一次；刷新按钮带 loading 态；
  - 无设备/接口报错：回退为原手输框，并在下方显示 `error` 文案与手动输入提示。
- 非 USB 类型保持原输入框不变；`form.source` 仍提交索引字符串（服务端 `capture_source` 已能转 int）。
- 样式用组件内 `<style scoped>` 或现有 utility class，**不要改** `theme.css/style.css/App.vue/main.js`。

### 3. 测试 `server/tests/test_usb_devices.py`

- 纯解析函数单测（把 stderr 文本作为输入）：
  - 2 个 video + 1 个 audio → 只返回 2 个、顺序正确；
  - 中文/带引号/带空格的设备名；
  - 无匹配行 → 空列表；
- ffmpeg 缺失（monkeypatch `Path.exists` 或 subprocess 抛 `FileNotFoundError`）→ 200 语义的空列表 + error；
- 缓存：连续两次调用只执行一次 subprocess；`refresh=1` 会再执行；
- API 层用 `TestClient`（参考 `server/tests/` 现有 app 构造方式），monkeypatch 枚举函数，
  断言 JSON 结构与鉴权开关两种口径。

### 4. 文档

- `docs/部署指南.md` §3.1 的 USB 行补充"添加摄像头页面可直接选择设备，无需记索引"；
- `docs/部署FAQ.md` 增加「USB 摄像头索引变了/找不到设备怎么办」（刷新、拔插、隐私挡板、
  关掉占用摄像头的会议软件、回退手输）。

## 约束

- 不改 `pipeline.py` 的 `capture_source`（任务 56 已完成）；不引入新依赖（标准库 + 现有 fastapi）；
- 不实际打开摄像头、不做推理；测试不占 8000/8001（TestClient 即可）；
- 前端改动仅限 `Cameras.vue`（必要时 `web/src/api.js` 加一个 GET 封装）。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 passed + 新增用例）；`cd web && npm run build` 通过；
- 本机实跑端点（8001 临时实例）能返回 HD Webcam / OBS Virtual Camera（以实际枚举为准），
  `refresh=1` 生效；ffmpeg 路径打不开时返回 200 + error；
- 前端 USB 选择器在设备列表为空时不崩、可手输；
- 报告附端点实跑 JSON 与 build 输出，不编造设备名。

## 执行报告

（待填：改动文件 / 实跑 JSON / 测试与 build / 遗留）
