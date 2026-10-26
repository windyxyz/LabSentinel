# 部署与运维 FAQ（任务 24）

> 全部条目来自本项目开发过程中**实际遇到并解决过**的问题（2026-10-25/27）。
> 格式：**现象 → 原因 → 解决**。查不到 here 的问题先看 [80-risk-register.md](80-risk-register.md)。

## 1. 安装与网络

**Q：pip 安装极慢/断流**
→ 国内直连 PyPI 不稳。解决：一律加清华源
```bash
pip install <pkg> -i https://pypi.tuna.tsinghua.edu.cn/simple
```

**Q：GitHub 下载（模型/二进制）失败或 SSL 错误**
→ 直连不稳。解决（按优先级尝试）：
```bash
# ① 镜像前缀 + 断点续传循环
for i in 1 2 3 4 5; do curl -sL --max-time 120 -C - -o <文件> "https://ghfast.top/https://github.com/<路径>" && break; done
# ② 部分包走官方 CDN（如 OpenH264 用 ciscobinary.openh264.org）
# ③ GitHub API（api.github.com）通常可达，可先查资产真实 URL
```

**Q：npm 安装慢**
```bash
npm install --registry=https://registry.npmmirror.com
```

**Q：Roboflow 下载/上传连不上**
→ 需要代理且注意参数名。解决：`HTTPS_PROXY=http://127.0.0.1:7890`（本地代理不一定在线，先探测）；
Roboflow API 的 key 参数名是 **`api_key`**（不是 key/token）。key 一律走 `RF_API_KEY` 环境变量，禁止写死。

**Q：HF（HuggingFace）不可达**
→ 用 hf-mirror 镜像：`export HF_ENDPOINT=https://hf-mirror.com`

## 2. 依赖冲突（最高频事故区）

**Q：推理报错/onnxruntime 提供者消失**
→ **onnxruntime（CPU 版）与 onnxruntime-directml 同装互杀**（ultralytics 的 AutoUpdate 也会偷偷装 CPU 版！）。
解决（两个一起卸再装回）：
```bash
pip uninstall -y onnxruntime onnxruntime-directml
pip install onnxruntime-directml==1.24.4 -i https://pypi.tuna.tsinghua.edu.cn/simple
```
预防：`tools/export_onnx.py` 已内置 `YOLO_AUTOINSTALL=false`；npu-venv 里 CPU 版与 vitisai 同理互杀。

**Q：numpy 报二进制不兼容 / import 崩溃**
→ numpy 1.x/2.x wheel 按版本编译，混装即崩。解决：m0-venv **numpy 不许动**；npu-venv 钉 **numpy 1.26.4**（AMD wheel 按 1.x 编译）、pandas 钉 **2.2.3**。

**Q：ByteTrack 报错/找不到跟踪器**
→ **supervision 必须 0.25.1**（0.26+ 把跟踪器移出主包）：
```bash
pip install supervision==0.25.1 -i https://pypi.tuna.tsinghua.edu.cn/simple
```

**Q：pytorch.org 下载 torch 超时**
→ pytorch 官方源本网络不可达，直接用 PyPI 的 CPU 版 torch（清华源）。

**Q：pip 安装报 WinError 5 拒绝访问（onnxruntime DLL）**
→ 运行中的 uvicorn/训练进程锁着 DLL。解决：先停服务（scripts\stop-all.bat）再装；
纯 Python 包（如 python-multipart）无 DLL 锁风险可例外。

## 3. 运行与端口

**Q：端口被占用启动失败**
→ `scripts\start-all.bat` 会自动检测 1984/8000/5173 并**跳过**被占用组件；手工排查：
```bash
netstat -ano | findstr ":8000" | findstr "LISTENING"   # 最后一列是 PID
taskkill /PID <pid> /F
```
或直接 `scripts\stop-all.bat`（带确认）。

**Q：vite 起了但 curl http://127.0.0.1:5173 不通**
→ vite dev **仅绑 IPv6 localhost**。解决：`curl -g "http://[::1]:5173"`；浏览器用 http://localhost:5173 无碍。

**Q：Git Bash 的 curl 发中文 JSON 后服务端收到乱码**
→ Git Bash 转码损坏。解决：测试 API 一律用 python httpx 写脚本。

**Q：浏览器打开事件视频无法播放**
→ 缺 OpenH264 编码库（服务器端生成时回退了 mp4v）。解决：下载 `openh264-2.5.0-win64.dll`
（Cisco 官方 CDN：`http://ciscobinary.openh264.org/openh264-2.5.0-win64.dll.bz2`，解压得 dll）放入
`m0-venv\Lib\site-packages\cv2\`——`server/app/__init__.py` 已自动把该目录加进 DLL 搜索路径。

## 4. 模型与推理

**Q：多路摄像头时 worker 全部静默死亡，甚至整机死机**
→ **DirectML 会话跨线程并发 Run 不线程安全**（本项目最大事故，详见 51-loadtest-report.md §3）。
已修复：`engine.py` 模块级 `_ACCEL_LOCK` 全局串行 + worker 单帧异常自愈。
规则：**任何推理必须走 `Engine._run()`，代码出现 `sess.run` 即不合格**；训练/压测/批量推理禁止并行。

**Q：NPU 推理怎么弄**
→ XINT8 量化（检测头保留 FP32，`/model.23` 子图排除——全图量化类别输出会归零）；
本机无 MSVC 需走 `tools/quantize_npu.py` 内置的"注入绕道"；配方细节见 tools/README.md 第二节。

**Q：PPE 模型换了之后检测类别不对**
→ 词表顺序必须与模型输出一致。PpeEngine 优先读模型同名 `<model>.classes.txt`（一行一类），
没有才回退 `LS_PPE_CLASS_NAMES`。换模型：导出 ONNX → 生成 classes.txt → 改 `.env` 的 `LS_PPE_MODEL` → 重启。

**Q：白大褂判定不准/误报**
→ 采用**证据分级**：No Lab Coat 显式检测（conf≥0.30）才算违规；Lab Coat 低阈值（0.20）作合规证据；
两者皆无 = "不确定"（不告警）。阈值可调 `LS_PPE_CONF`/`LS_PPE_WEARING_CONF`。持续不准走数据飞轮（docs/90）。

## 5. 训练

**Q：训练命令与安全**
→ 限线程+过夜，完整前置条件与看门狗见 [50-tasks/训练安全规程.md](50-tasks/训练安全规程.md)。速记：
```bash
D:/LabSentinel/m0-venv/Scripts/python.exe tools/train_ppe.py --data datasets/merged/data.yaml \
    --epochs 24 --batch 8 --workers 2 --name ppe_v2
```
本机在高负载组合下（CPU 满核+GPU 推理+虚拟显示器/剪映）**发生过三次整机硬挂起**
（Kernel-Power 41，DML 并发已修复但硬件侧建议照做：更新 BIOS/驱动、停用 Parsec/GameViewer 虚拟显示器）。

**Q：训练中断了怎么办**
→ 看项目根 `.training.lock`（记录 PID）与 `runs/detect/<name>/weights/last.pt` 是否更新；
规程约定**不自动重启**，人工确认后可 `--exist-ok` 续跑或重跑。

**Q：GPU 训练可行吗**
→ 可选路线：`D:\LabSentinel\gpu-venv`（Python 3.12 + torch-directml 0.2.5）+ `--device dml`。
注意 torch-directml 只发到 cp310/cp311 wheel；实测加速比与稳定性见 tools/README.md 第八节。

## 6. 数据与标注

**Q：Roboflow 导出的 YOLO 标签一行几十个数字**
→ 那是**分割多边形格式**，不是检测框（本项目实测 4/5 的框因此被静默丢弃）。解析代码必须兼容：
>5 字段且偶数个 → 按 x/y 序列取外接矩形。`tools/eval_report.py` 的 `load_gt` 可参考。

**Q：标注规范在哪**
→ [90-data-flywheel.md](90-data-flywheel.md) 第二节（Lab Coat/No Lab Coat 语义、遮挡阈值、歧义帧处理）。
数据集许可 CC BY 4.0；对外发布前必须过 `tools/export_dataset.py` 生成的脱敏确认清单。

## 7. 告警与推送

**Q：各渠道 key 从哪拿**
→ [部署指南](部署指南.md)「4. 告警渠道配置」表格（企微/钉钉/飞书/Server酱/PushPlus/Qmsg/SMTP 逐个说明）。

**Q：为什么发不出/被拒绝**
→ 安全机制：域名白名单 + 强制 https + 不跟随重定向（防 SSRF）。钉钉/企微 HTTP 200 但
body `errcode≠0` 也判失败（`_is_ok`）。失败会重试（1s/4s/15s 三次）→ 死信 →「摄像头」页告警面板手动重发。

**Q：怎么验证渠道通了**
→ 「摄像头」页告警面板「发送测试告警」按钮，或 `POST /api/alert-test`（不落 events 表）。

## 8. 前端

**Q：bundle 体积预算**
→ 项目体积敏感，禁止引 ECharts 等重型库；图表全部原生 SVG（Reports.vue 可参考）。
当前 gzip 约 42KB。

**Q：主题令牌在哪/我能改全局样式吗**
→ WinUI 3 令牌在 `web/src/theme.css`（浅/深双主题）；**该文件与 style.css/App.vue/main.js 为主线专属**，
其他协作者新增样式一律写组件 `<style scoped>`，令牌（--card-bg/--accent/--l1~l3/--stroke-*）直接复用。

**Q：写接口 401**
→ 服务端启用了管理鉴权（LS_ADMIN_TOKEN）。「摄像头」页顶部填写令牌（存浏览器 localStorage），
所有 POST/DELETE 自动携带。
