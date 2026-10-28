# 任务 47 · 便携版首次运行播种（演示摄像头/视频/区域规则）

> 背景：任务 44–46 已让便携包"双击即起"，但实测首次启动是**空库、且包里没有演示视频**（`dist/portable/LabSentinel` 下无 mp4）。用户打开看到空仪表盘，不符合"开箱即用"。
> 目标：便携包自带两段演示视频；首次启动自动播种 4 路演示摄像头 + 区域 + 规则；清空数据后再次启动也能恢复演示状态。

## 目标

1. **构建器带演示视频**（改 `scripts/build_portable.py`）：
   - 复制 `m0-spike/data/lab-walk-24076.mp4`、`lab-equip-21453.mp4` 到 `DIST/media/`（约 8.4MB）；
   - 校验文件存在与大小；构建报告加 media 体积。
2. **首次运行播种**（改 `scripts/portable_launcher.py`）：
   - 首次创建 `%LOCALAPPDATA%\LabSentinel\data\labsentinel.db` 后（或在启动服务前），若 cameras 表为空：
     - 插入 4 路文件摄像头：`lab-slice/ppe-regress/ppe-final` → `media\lab-walk-24076.mp4`，
       `ppe-equip` → `media\lab-equip-21453.mp4`（绝对路径指向便携包 media）；
     - 每路 1 个安全区 + `enter`(L2)/`enter_no_ppe`(L3) 规则（沿用 `scripts/deploy.py` 的 SEED_ZONES 坐标）；
     - 写 `LS_FILE_LOOP=true`，摄像头播完循环。
   - 播种只在**空库**执行；用户已有数据时绝不覆盖。
   - 若 `media/` 缺失，跳过播种并在日志警告（不阻塞启动）。
3. **重建便携包并实测**：
   - `python scripts/build_portable.py --zip`；
   - 删除/改名 `%LOCALAPPDATA%\LabSentinel`（先备份到 `LabSentinel.bak`，测完恢复或删除）；
   - `PATH=C:\Windows\System32` 启动便携包 → UI 显示 4 路摄像头；启动其中 1 路 → fps>0、出事件、截图 200；
   - 停止；再次启动（已有数据）→ 不重复播种（摄像头仍 4 路）。
4. **文档/报告**：更新 `docs/59-portable-install.md`（首次运行会播种演示数据、数据位置、如何清空）；执行报告写命令/证据/体积。

## 约束

- 只改 `scripts/build_portable.py`、`scripts/portable_launcher.py`、相关文档；不改服务端业务代码；
- 不覆盖用户数据；播种失败不阻塞启动（警告即可）；
- 不训练；测试用隔离目录，结束后恢复到测试前状态（备份）。
- 不把 `server/data`、事件库、截图打进包；只带两段演示 mp4。

## 验收标准

- 便携包 `media/` 含两段 mp4；新包体积合理（+~8MB）；
- 清空数据后首次启动：4 路摄像头/区域/规则齐全，启动一路能出事件与截图；
- 二次启动不重复播种；用户已有数据不被覆盖；
- 文档与报告齐全；`pytest` 不受影响（无服务端业务改动）。

## 执行报告

（待填：构建改动 / 首次播种证据 / 二次启动检查 / 体积 / 数据恢复确认）

### 执行报告（flash，2026-10-28 07:55 交付）

- **构建改动**：`build_portable.py` 复制两段演示视频 → `DIST/media/`（含存在性与 >1MB 校验；
  体积报告行同口径）。
- **启动器改动**：`portable_launcher.py` 新增 `seed_if_empty()`（sqlite3 直接建表+播种，schema 与
  server/app/store.py 同步）：仅当 cameras 表为空且 media 齐全 → 播 4 路文件摄像头（绝对路径指向
  包内 media）+ 每路 1 区 2 规则；media 缺失只警告；`first_run()` 在建目录后调用。
- **实测**（备份 LOCALAPPDATA\LabSentinel → .bak，robocopy 到 ASCII 路径 D:\tmp\pt47，PATH=System32）：
  1. 首次启动：**4 路摄像头全部出现**（/api/system）；
  2. 启动第 1 路：60s 内 **14 条 enter 事件**、截图 `200 image/jpeg` ✓；
  3. 停止 → 二次启动：**cameras 仍 4 路，无重复播种** ✓；
  4. 手动删光 cameras 再插 1 路 `user-cam` → 重启后仅 1 路 user-cam，**用户数据不被覆盖** ✓；
  5. 清理：测试目录/测试数据删除，备份已恢复。
- **体积**：media +8.4MB（两段 mp4），总 708MB → 716MB 量级。
- **遗留问题**：无。（播种状态为 stopped，用户在 UI 点启动；LS_FILE_LOOP=true 由 config.env 带入。）
