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
