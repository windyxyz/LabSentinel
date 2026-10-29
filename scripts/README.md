# scripts/ · 启动/停止/健康检查/部署脚本（任务18 + 2026-10-27 扩展）

| 脚本 | 作用 | 说明 |
|---|---|---|
| `..\一键启动.bat`（项目根） | **交互式一键启动器**（日常用这个） | 启动前先跑 `deploy.py check --quiet`，缺什么自动补（venv/pip/npm/ffmpeg/OpenH264/配置/演示数据）；然后探测服务：已运行→直接打开控制面板；未运行→调 start-all 拉起并等待就绪（最多 45 秒）→自动打开面板→进入菜单。菜单 [7] 重部署保留数据 / [8] 重部署清空数据 / [9] 一键卸载。环境变量 `LS_LAUNCHER_NO_BROWSER=1` 禁止自动弹浏览器 |
| `deploy.py` | **部署引擎**（纯标准库，无 venv 也能跑） | `check` 环境体检 9 项；`install` 自动部署（venv→pip→OpenH264→node→npm→ffmpeg→.env→播种演示库）；`redeploy --keep-data/--wipe`；`uninstall`（数据改名备份）。下载全走国内镜像（pypi 清华/npmmirror），请求前校验仅公网 http/https；项目外 venv 只解除引用绝不删除 |
| `make_demo_zip.py` | 打 demo 分发包 | 产物 `dist/LabSentinel-demo-<日期>.zip`（约 94MB）：源码/模型/演示视频/go2rtc/文档；排除数据集、训练产物、venv、node_modules、ffmpeg、server/.env（含机器路径）、server/data（新机播种） |
| `start-all.bat` | 一键启动 go2rtc(1984) + 服务端(8000) + vite(5173) | venv 路径从 `scripts\venv_path.txt` 动态读取（deploy.py 写入，**GBK 编码**——cmd for /f 按系统代码页读，UTF-8 中文路径会乱码）；端口被占用则**跳过**该组件；环境预检；日志落 `logs/<name>.log`；尾部显式 `exit /b 0`（否则末尾端口检测的 findstr 会把 errorlevel 置 1，call 方误判失败） |
| `stop-all.bat` | 一键停止 | 按端口找 PID `taskkill /F`，带确认；顺带关闭名为 go2rtc/server/web 的控制台窗口壳 |
| `health-check.bat` | 状态表 | curl 探测 API/system、go2rtc /api、Web（vite 仅绑 IPv6，用 `[::1]` 探测） |
| `release_manifest.py` | 发布产物清单与校验和（任务58） | 对 dist/ 最新 zip 生成 `SHA256SUMS.txt` + `RELEASE-MANIFEST.json`（version/size/sha256/条目数/缺失项/git）；`--verify dist` 重算对比；必需条目缺失 exit 2 |
| `smoke_portable.py` | 便携包一键冒烟（任务59） | `python scripts/smoke_portable.py --zip dist/LabSentinel-portable-*.zip` → 解压到 %TEMP%、沙箱 LOCALAPPDATA、启动（exe 优先/启动.bat 回退）→ 就绪/前端/播种 8 项 PASS/FAIL → `--stop` → 隔离断言（真实 %LOCALAPPDATA% 不变）→ 清理；失败 exit 1 并保留现场；`--dir` 调试已解压目录；`--port` 默认 8180 不漂移 |

## 用法

```bat
一键启动.bat              :: 日常入口：体检→自动部署→探测→按需启动→开面板→菜单（双击即可）
python scripts\deploy.py check     :: 环境体检（9 项）
python scripts\deploy.py install   :: 仅部署不启动
python scripts\make_demo_zip.py    :: 打 demo 分发包
scripts\health-check.bat  :: 状态表 [OK]/[DOWN]
scripts\stop-all.bat      :: 停止（Y/N 确认）
```

## 已知坑

1. **编码**：三个 .bat 必须是 **GBK/ANSI 编码**（cmd 按 OEM 代码页解析批处理；UTF-8 保存会把中文
   echo 变成乱码甚至破坏语法）。用编辑器修改后请转回 ANSI 再保存。
2. vite dev 仅绑 IPv6 localhost（README 已知事项），health-check 用 `[::1]:5173` 探测。
3. `stop-all.bat` 会停掉**所有**占用 1984/8000/5173 的进程——若有其他服务误用这三个端口请先挪开。
4. 不自动启动训练/推理（训练由巡检按《训练安全规程》管理）。
