# 任务 45 · 便携启动器与首次运行（无黑框、单实例、数据外置）

> 背景：任务 44 产出了可运行的便携目录；本任务做用户真正双击的入口：设置数据目录、选端口、等就绪、开浏览器、单实例、可停止。

## 目标

1. **新增 `scripts/portable_launcher.py`**（只用标准库，由 `pythonw.exe` 运行）：
   - 定位 `APP_ROOT = 脚本上一级`（便携包根）；`RUNTIME = APP_ROOT/runtime`；`SERVER = APP_ROOT/app/server`。
   - **数据目录**：`%LOCALAPPDATA%\LabSentinel\`，首次创建：
     - `data\`（db/snapshots/videos/annotation_pool）、`logs\`、`config.env`
     - 首次从 `app/server/.env.template` 复制到 `config.env`，把模型路径改成 `APP_ROOT\models\...` 的绝对路径。
   - **环境变量**：`LS_DB_PATH/LS_SNAPSHOT_DIR/LS_VIDEO_DIR/LS_ANNOTATION_POOL_DIR` 指向数据目录；
     `LS_PPE_MODEL` 指向 models；`LS_WEB_DIST` 指向 `app/web`；`LS_FILE_LOOP` 读 config。
   - **端口**：优先 config 里的 `port`（默认 8000）；被占用则 8100–8199 逐个试 bind；选定后写回 `config.env`。
   - **单实例**：`logs\launcher.lock` 写 PID；启动前检查锁+PID 存活/健康检查；
     若已在运行 → 直接打开浏览器退出，不重复起服务。
   - **启动**：`subprocess.Popen([runtime\python.exe, "-m", "uvicorn", "app.main:app", "--port", str(port)], cwd=SERVER, env=..., creationflags=CREATE_NO_WINDOW)`；
     stdout/stderr 重定向到 `logs\server-YYYYmmdd.log`。
   - **就绪等待**：轮询 `http://127.0.0.1:<port>/api/system`，最多 30s；成功 → `webbrowser.open`；失败 → 写日志并提示（可弹 MessageBox 用 ctypes，可选）。
   - **停止**：`--stop` 参数读锁文件 PID，`taskkill /PID`（先 terminate，2s 后 kill）；退出码 0。
2. **入口脚本**（便携包根，GBK 编码）：
   - `启动.bat`：`start "" "%~dp0runtime\pythonw.exe" "%~dp0scripts\portable_launcher.py"`（无窗口；bat 自身一闪而过）
   - `停止.bat`：`"%~dp0runtime\python.exe" "%~dp0scripts\portable_launcher.py" --stop`
   - 若 pythonw 不可用，回退 `python.exe`（会留一个控制台窗口，需在报告说明）。
3. **源目录同步**：把 `portable_launcher.py`、`启动.bat`、`停止.bat` 也放进 `scripts/`（源码仓库），构建脚本（44）负责复制到便携包根。
4. **实测**（在本机模拟干净环境）：
   - 用任务 44 的便携目录；`set PATH=C:\Windows\System32` 后双击/命令行运行 `启动.bat`；
   - 验证：无黑框（pythonw）、浏览器自动打开、`/api/system` 正常、数据写进 `%LOCALAPPDATA%\LabSentinel`；
   - 再次运行 → 不重复起进程，直接开浏览器；
   - `停止.bat` → 进程退出，端口释放；日志落盘。
5. **报告**：命令、端口选择、数据目录树、单实例/停止验证、日志样例、已知问题。

## 约束

- 只用标准库；不引入 pystray/第三方托盘（阶段 2 再议）。
- 不改业务代码/主线样式；不训练；不占 8000/5173（测试用默认 8000 也可，但测试完必须停）。
- 不删除用户数据；`--stop` 只停进程。
- 启动器必须能在「系统没有 Python/Node」的 PATH 下工作（只用 runtime）。

## 验收标准

- `PATH=System32` 下 `启动.bat` 能拉起 UI（单端口），无 python 控制台窗口；
- 数据目录、config、日志按预期生成；模型路径正确；
- 二次启动不重复起服务；`停止.bat` 能停；
- 端口冲突时自动换端口并在浏览器打开正确端口；
- 报告含实测记录与日志样例。

## 执行报告

（待填：端口/数据目录 / 无黑框与单实例验证 / 停止验证 / 日志样例 / 已知问题）

### 执行报告（flash，2026-10-28 02:20 交付）

- **新增**：
  - `scripts/portable_launcher.py`（纯标准库，pythonw 运行）：数据外置 `%LOCALAPPDATA%\LabSentinel\`
    （首次从 .env.template 生成 config.env 并把模型路径改绝对）；env 注入
    `LS_DATA_DIR/LS_DB_PATH/LS_SNAPSHOT_DIR/LS_VIDEO_DIR/LS_ANNOTATION_POOL_DIR/LS_PPE_MODEL/
    LS_MODEL_FP32/LS_WEB_DIST` + config.env 里 LS_* 透传；端口 config `port`（默认 8000）被占则
    8100–8199 试 bind 并写回；单实例 `logs/launcher.lock` 记 "pid port"；uvicorn 子进程
    CREATE_NO_WINDOW、日志 `logs/server-<日期>.log`；就绪轮询 30s → webbrowser.open（失败弹
    MessageBox）；`--stop` 按锁 PID taskkill 后删锁。
  - `scripts/portable_start.bat` / `portable_stop.bat`（内容纯 ASCII，构建时复制为包根
    `启动.bat`/`停止.bat`——中文文件名不影响 cmd，内容零 GBK 依赖）；pythonw 缺失回退 python.exe。
  - `build_portable.py`：+入口复制；模板补 `LS_MODEL_FP32`（便携布局下 Engine 默认路径不存在，
    首次实测 /api/system 500 即此因）；冒烟后自动清理 config 导入期产生的空 data 目录。
- **实测**（便携目录复制到纯 ASCII 路径 D:\tmp\pt + `set PATH=C:\Windows\System32`，5 步全过）：
  1. 首次启动：bat rc=0、pythonw 无黑框（tasklist 可见）、**pid=21272 port=8000 health=OK**；
  2. 数据外置：data/logs/config.env 全落 LOCALAPPDATA，模型路径绝对化 ✓；
  3. 二次启动：锁不变（pid/port 均同）→ 不重复起服务 ✓；
  4. 端口冲突：停实例→占住 8000→重启→**自动切 8100**（pid=15432）✓；
  5. `停止.bat`：进程退出、端口释放、锁删除 ✓。
- **关键坑（已修）**：`pid_alive` 原用 tasklist 输出解析——tasklist 输出是 ANSI/GBK，Python 跑在
  UTF-8 模式时读取线程 UnicodeDecodeError → stdout 空 → **活进程被判死 → 单实例失效起了第二实例**
  （02:13 实测翻车）。改为 **ctypes OpenProcess** 判活（无编码依赖）后 5 步全过。
- **遗留问题**：
  - 无系统托盘（约束只用标准库；阶段 2 再议 pystray）；
  - 停止.bat 用 python.exe 会闪一个控制台窗口（按任务规格保留，pause 需按任意键）；
  - 启动失败仅 MessageBox+日志，无重试。
