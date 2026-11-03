# 任务 122 · 便携包重建（含 119/120/121）+ 冒烟/manifest/doctor（P0，发布链）

> 背景：当前 `dist/LabSentinel-portable-20261031.zip` 是 119/120 时代的包，**不含任务 121
> 图片文件夹自动识别**。在下一次 VM/RC/发布前必须重建，并用 58/59/70 脚本留下证据。
> 这是后续 118（干净 VM）与 104（最终 RC）的前置。

## 目标

1. 用当前代码（HEAD `f7ba376`）重建便携包，产物包含：
   - `app/server/app/folders.py`、`vision.py`（121）；
   - `folder_images` schema v5（首次启动自动迁移）；
   - 前端 `web/dist` 已含「图片识别」页（构建时间需晚于 `f7ba376`）。
2. 生成 manifest/SHA256 并校验通过。
3. 冒烟脚本 8/8 PASS，冷启动 < 60s（119 的 `LS_READY_TIMEOUT_S` 默认 60）。
4. 包内 `doctor.py` FAIL 0。
5. 更新 `docs/发布核对报告.md` 的包信息/测试数字（pytest **329**、build gzip **54.29KB**）。
6. 清理临时进程/端口/沙箱，真实 `%LOCALAPPDATA%` 不被冒烟污染。

## 步骤（严格按序）

```powershell
cd D:\User\Desktop\TOOLS\监控识别
# 0) 确认工作区干净（不要提交 .zcode/）
git status --short
git log --oneline -1          # 应为 f7ba376 或其后

# 1) 前端构建（若 dist 不是最新；会改 web/dist，已被 gitignore）
cd web; npm run build; cd ..

# 2) 重建 zip（幂等；runtime 已存在会跳过下载）
python scripts\build_portable.py --zip

# 3) manifest + 校验
python scripts\release_manifest.py --zip dist\LabSentinel-portable-<日期>.zip
python scripts\release_manifest.py --verify dist

# 4) 冒烟（默认 --timeout 90，勿低于 60）
python scripts\smoke_portable.py --zip dist\LabSentinel-portable-<日期>.zip

# 5) 包内 doctor（解压到临时目录或 dist/portable/LabSentinel）
python dist\portable\LabSentinel\scripts\doctor.py --root dist\portable\LabSentinel

# 6) 快速 RC（如脚本可用）
python scripts\rc_regression.py --fast
```

> Python 用项目 `.venv` 或 `D:\LabSentinel\m0-venv` 均可；**不要**在 VM 内构建/冒烟。

## 验收证据（写进报告）

- 新 zip 文件名/大小/条目数/sha256；
- manifest `--verify` 结果；
- smoke 输出（8/8 + 就绪秒数 + backend）；
- doctor 输出（PASS/WARN/FAIL 计数）；
- `git log -1` 与 `npm run build` gzip；
- 端口 8180/8000/1981/1984 无残留监听、无 pythonw/uvicorn 残留；
- 明确记录“121 图片文件夹功能已包含在包内”（可解压检查 `app/server/app/folders.py`）。

## 约束

- **不改业务代码**；若构建/冒烟失败，只做最小构建脚本修复并写报告（修复需 pytest 回归）；
- 不跑真实推理压测/24h/训练；不在 VM 内运行；
- 不上传/不发布/不推 tag；不提交 `dist/` 二进制（按仓库现状）；
- 冒烟失败保留现场日志，先定位再重跑；清理 `%TEMP%\ls-smoke-*`。

## 完成后

- 在本文末尾写 `## 执行报告`（命令、数字、失败与修复）；
- 把 zip 名/sha256 写进 `docs/发布核对报告.md`；
- 下一项：118（Hyper-V 干净 VM 复测，使用本包；低负载，人工协作，失败写 BLOCKED）。

## 执行报告

**flash 2026-11-03 完成（HEAD f07c0df 重建；smoke 8/8、doctor 10/0/0、rc_regression 4/4、冷启动 20.0s）**

- **产物**：`dist/LabSentinel-portable-20261103.zip`——278.9MB / **12598 条目** /
  sha256 `6073b88f28c0935d1426aa533cc2f6c1689638f4e7665e7c276f8ceb3a58c5d9`
  （git=2f0403a；**含 121**：folders.py/vision.py/图片识别页 index-9gf8y74S.js 解压核验 √；
  也含 106-108 主线修复，f07c0df 在 f7ba376 之后）。
- **步骤实测**：npm build ✓（gzip **54.29KB**，与看板口径一致）；build_portable --zip ✓
  （app 体积 3→4MB）；manifest --verify **OK**（必需条目齐全，可选缺卸载.bat=安装器生成属预期）；
  smoke **8/8 PASS**（backend=directml、播种 4 路、真实 LOCALAPPDATA 零污染、新资产 200）；
  包内 doctor **10 PASS / 0 WARN / 0 FAIL**；
  **冷启动直证 20.0s**（隔离解压→`/api/system` 200，窗口 60s，余量充足）；
  **rc_regression --fast 4/4 PASS exit 0**（pytest PASS @主线口径 343 / build 1.5s /
  release_check 0.9s / doctor 0.1s）。
- **清理**：8180/8185/8187/1981 无监听、pythonw/LabSentinel.exe 零残留、`%TEMP%\ls-smoke-*`
  与 ls122-* 已清、主库 mtime 10-31 未动。
- **遗留**：无。下一项 118（Hyper-V 复测用本包）。
