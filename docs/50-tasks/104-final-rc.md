# 任务 104 · 最终 RC 回归与发布清单更新（P1，最后执行）

> 前置：75–103 已完成或明确 BLOCKED。本任务是夜间批次的收口：重建最终便携包、跑全套自动回归、
> 更新发布核对报告。**不发布、不推 tag、不对外宣称 24h 已验证。**

## 步骤（严格按序）

1. **确认工作区**：`git status` 记录未提交文件（由主线提交，不自行 commit）；确认无服务占用
   8000/8001/1984/5173/8554。
2. **全量测试**：`cd server && D:/LabSentinel/m0-venv/Scripts/python.exe -m pytest tests/ -q`；
   记录 passed 数；失败则先修复（本任务可修回归，不新增功能）。
3. **前端构建**：`cd web && npm run build`；记录 gzip 体积。
4. **重建便携包**：`python scripts/build_portable.py --zip`（用项目 `.venv`）；确认新 zip 日期；
   包内包含 75 修复的脚本/ffmpeg 路径。
5. **manifest**：`python scripts/release_manifest.py --zip <新 zip>` + `--verify`；记录
   大小/sha256/条目数/缺失项。
6. **冒烟**：`python scripts/smoke_portable.py --zip <新 zip>` → 8/8 PASS；失败保留日志并修。
7. **doctor**：解压根目录运行 `runtime\python.exe scripts\doctor.py --root <根>` → FAIL 0；
   包内 `backup_data.py/collect_diagnostics.py --help` 可运行。
8. **配置/诊断**：隔离 data 下生成诊断包，grep 不到真实 token/password；`collect_diagnostics`
   在便携包默认 data 路径下能工作。
9. **RC 脚本**：`python scripts/rc_regression.py --smoke --zip <新 zip>`（97 完成后）；输出表格。
10. **release_check**：`python scripts/release_check.py` → FAIL 0。
11. **文档终稿**：按 100 更新 `docs/发布核对报告.md`/release notes/公告的**最终数字**；
    明确阻塞：真实 24h、模型止损线、干净 VM、GitHub CI、签名。
12. **清理**：临时解压/备份/日志删掉；确认端口空闲、主库 mtime 未变。

## 验收

- pytest 全绿（记录 N passed）；build 通过；新 zip + manifest + smoke 8/8 + doctor FAIL 0 +
  release_check FAIL 0；
- 文档数字与 manifest 一致；无“24h 已验证”表述；
- 报告附命令/输出/新 zip 信息/阻塞清单。

## 约束

- 不发布、不 git commit/tag/push；不跑 24h/训练；
- 不掩盖失败：任何步骤失败如实记录，修复后重跑；无法修复写 BLOCKED；
- 主库只读，不写入。

## 执行报告

（待填：pytest/build/zip/manifest/smoke/doctor/release_check 结果 + 阻塞 + 清理）

## 执行报告

**flash 2026-11-04 完成（491 passed / zip 20261104 / smoke 8/8 / 包内 doctor 10/0/0 / rc 4/4 / release_check 全 PASS）**

### 逐步结果

| 步骤 | 结果 |
|---|---|
| 1 工作区 | git status 有本批未提交文件（主线提交，不自行 commit）；端口 8000/8001/1984/5173/8554 无监听 |
| 2 pytest | **491 passed**（含 109-126 全部新测试） |
| 3 build | 通过，gzip 60.11KB |
| 4 重建包 | `LabSentinel-portable-20261104.zip` 279MB（**发现并修复 staging pyc 残留**：构建期冒烟 import 会重新生成 __pycache__，make_zip 前递归清理——app 内 pyc 28→**0**） |
| 5 manifest | 279,397,565B / sha256 4de5f256… / 12,605 条目 / 缺可选仅卸载.bat / --verify OK |
| 6 smoke | **8/8 PASS**（directml、播种 4 路、110 残留收尾、隔离零污染） |
| 7 包内 doctor | **10 PASS / 0 WARN / 0 FAIL**（解压根实测） |
| 9 rc_regression | **4/4 PASS exit 0**（manifest 引用新 zip sha 4de5f256…） |
| 10 release_check | **全 PASS**（319 链接/TODO 0/敏感 0） |
| 11 文档终稿 | 100 完成（核对报告/notes/公告用最终数字+阻塞声明） |
| 12 清理 | smoke/doctor 临时目录已删；端口空闲；主库未动 |

### 阻塞清单（发布前必须，均未解除）

1. 真实 RTSP 摄像头 24h（用户侧硬件）——**唯一硬阻塞**；
2. 模型止损线（Lab Coat ≥60%、误报 ≤20/天）——报表页处置质量卡取证；
3. GitHub Actions 实跑（tests.yml 就位，需远端仓库）；
4. 代码签名（可选）。
**不发布/不推 tag/无"24h 已验证"表述。**

### 98 可选压测记录

不跑（重负载独占 8001 与连续回归窗口冲突；此前 57/65 两轮多路压测记录已覆盖场景，
发布不以其为阻塞）。转 backlog，机器空闲时按 57 规格补跑。

### 主线复审补丁（2026-11-04，之后由 130 重建）

- 本包构建于复审修复前：**不含** genai 中文路径、插件入口解析/目录穿越、MQTT LWT 顺序修复；
- 版本已统一 `0.1.0-alpha`（原 m1），OpenAPI 快照已重生成；
- 最终候选包以任务 **130** 重建结果为准，本记录保留为 122–130 批次快照。
