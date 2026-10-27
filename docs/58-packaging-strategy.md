# 58 · 开箱即用打包方案（服务端 + 客户端环境问题）

> 背景：当前 demo 包虽然能「一键启动」，但首次仍要联网装 Python 依赖、Node、ffmpeg，本质还是开发环境。
> 目标：像普通软件一样——**双击就能用、不需要用户装 Python/Node、断网也能跑、数据与卸载可控**。
> 参考路径：先做便携绿色版（社区可分发），再做安装包（快捷方式/卸载），最后考虑桌面壳与移动端安装包。

## 一、可选方案对比

| 方案 | 优点 | 缺点 | 结论 |
|---|---|---|---|
| A. PyInstaller / Nuitka 单文件 exe | 交付物最像"一个程序" | onnxruntime-directml/OpenCV DLL 需要大量 hidden-import 与 hook；onefile 启动慢（解压到 temp）、杀软误报高；排障困难 | 不推荐作为首选 |
| **B. 嵌入式 CPython + 预装 site-packages + 启动器** | 无需编译器、DLL 原生在目录里、可解释可排障、可增量更新；对 ORT/OpenCV 最稳 | 体积大（安装后 ~700MB，zip ~350–450MB）；需要一个构建脚本 | **推荐** |
| C. Docker | 环境一致性最好 | Windows 上 DirectML/GPU 不可用（容器内基本只能 CPU）；要求用户装 Docker Desktop | 仅作为 Linux 服务器选项 |
| D. Tauri / WinUI WebView2 桌面壳 | 有独立窗口/托盘，像原生程序 | 仍是 WebView 渲染；与现有 Web UI 重复；客户端壳可后置 | 阶段 3 再评估 |
| E. 浏览器 + 本地服务 + 快捷方式 | 零客户端开发、现有 UI 直接用 | "像程序"程度取决于启动器/快捷方式/托盘 | 阶段 1/2 采用 |

## 二、推荐架构（便携绿色版）

```
LabSentinel/
├─ 启动.bat / LabSentinel.exe        # 启动器（阶段 1 用 bat+pythonw，阶段 2 可做托盘 exe）
├─ 停止.bat                          # 停止（阶段 2 可并入托盘菜单）
├─ runtime/                          # 嵌入式 Python 3.12（python-3.12.x-embed-amd64）
│  ├─ python.exe / pythonw.exe
│  ├─ python312._pth                 # 加入 Lib\site-packages
│  └─ Lib/site-packages/             # pip --target 预装的全部服务端依赖
├─ app/
│  ├─ server/                        # FastAPI 代码（不含 data/）
│  └─ web/                           # npm run build 产物（单端口由服务端托管）
├─ models/                           # yolo11n.onnx + yolo11n_ppe.onnx + classes.txt
├─ tools/                            # ffmpeg.exe / go2rtc.exe
├─ docs/ LICENSE NOTICE
└─ data/  → 首次运行重定向到 %LOCALAPPDATA%\LabSentinel（数据库/截图/视频/日志/配置）
```

关键点：
1. **单端口模式**：服务端直接托管 `web/dist`（同源，无 Vite/Node），启动后只需开 `http://127.0.0.1:<port>`。
2. **数据外置**：DB/截图/视频/日志/`.env` 放 `%LOCALAPPDATA%\LabSentinel`，程序目录只读也能跑；
   升级程序不丢数据，卸载可选保留/删除数据。
3. **启动器**：单实例锁；端口冲突自动换端口；设置环境变量（LS_DB_PATH 等）；等 `/api/system` 就绪后开浏览器；
   日志落 `%LOCALAPPDATA%\LabSentinel\logs\launcher.log`；pythonw 启动无黑框。
4. **模型/二进制随包**：v2 ONNX（11MB）、ffmpeg（79MB）、go2rtc（21MB）。ffprobe 可选（61MB，非必需）。
5. **离线可用**：构建时把依赖装进包里；用户侧不联网也能启动（告警推送/人脸等联网功能除外）。

## 三、体积与启动预估（实测口径）

| 组成 | 体积 |
|---|---|
| 嵌入式 Python 3.12 + 服务端 site-packages | ~600MB（其中 opencv/numpy/onnxruntime 占大头） |
| ffmpeg.exe | 79MB |
| go2rtc.exe | 21MB |
| 模型（2 个 ONNX + 词表） | ~22MB |
| 前端 dist | <1MB |
| **安装后合计** | **~700MB**；zip 压缩后约 350–450MB |
| 启动耗时 | 服务端就绪 ~2–5s（DirectML 初始化），浏览器打开总 ~3–6s |

## 四、分阶段落地（对应任务）

| 阶段 | 任务 | 产出 |
|---|---|---|
| 1. 单端口 | **43** | 服务端托管 web/dist + SPA 回退；不再需要 Node/Vite |
| 2. 便携运行时 | **44** | `scripts/build_portable.py`：下载 embed Python、pip --target、复制代码/模型/二进制，PATH 剥离冒烟 |
| 3. 启动器/首次运行 | **45** | 无黑框启动、单实例、端口选择、数据目录、日志、开浏览器、停止 |
| 4. 安装/卸载 | **46** | `安装.bat`/快捷方式/可选开机自启/卸载（保留或删除数据）、干净 PATH 验收 |
| 5. 发布流水线 | 47（后续） | CI 构建 portable zip + sha256 + Release 附件；干净虚拟机矩阵 |
| 6. 客户端壳 | 48（后续决策） | Tauri/WinUI WebView2/Flutter desktop 评估；Android APK 走 Flutter + CI |

## 五、风险与对策

- **杀软误报**：嵌入式 Python 目录比 PyInstaller onefile 温和；后续做代码签名（EV 证书）。
- **DirectML/VC 运行时**：embed 包自带 vcruntime；onnxruntime-directml 自带 DirectML.dll；在干净 Win10/11 验证。
- **路径/中文**：启动器统一用 `%LOCALAPPDATA%` ASCII 路径存数据；程序目录可含中文。
- **端口冲突**：启动器探测 8000→8100，写回配置；防火墙首次弹窗允许即可（本地回环不受影响）。
- **升级**：阶段 1 用"覆盖程序目录、保留 data"；阶段 2 可做版本检查与增量包。
- **许可**：Python/ffmpeg/go2rtc/模型许可随包附 NOTICE；AGPL 源码可获得性不变。

## 六、客户端结论（当前）

- **Windows**：阶段 1/2 的「便携包 + 快捷方式 + 本地 Web UI」就是开箱即用形态，不需要单独客户端环境。
- **Android**：APK 本身即安装包；Flutter 骨架已在 `android/`，后续用 CI 出 APK（需要 Flutter SDK，另立任务）。
- **iOS/HarmonyOS**：暂缓，等 M2 决策。
- 桌面原生壳（Tauri/WinUI）只在"必须有独立窗口/托盘/开机自启体验"时再做，避免重复维护一套前端。
