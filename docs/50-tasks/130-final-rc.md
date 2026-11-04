# 任务 130 · 最终 RC 重建（P0，批次 130–139 收口）

> 前置：131–136 已完成或明确 BLOCKED。当前 `LabSentinel-portable-20261104.zip` 构建于复审修复前，
> **不含** genai 中文路径/插件入口校验/MQTT LWT/版本统一等修复，必须重建最终候选包。
> 本任务不发布、不推 tag、不宣称 24h 已验证。

## 步骤（严格按序）

```powershell
cd D:\User\Desktop\TOOLS\监控识别
git status --short            # 应只有主线待提交/已提交后的干净状态；flash 不 commit
git log --oneline -1

# 1) 全量回归
cd server; D:\LabSentinel\m0-venv\Scripts\python.exe -m pytest tests/ -q; cd ..
cd web; npm test; npm run build; cd ..

# 2) 重建前清理 staging（build 内部已防御 pycache，仍建议）
python scripts\build_portable.py --zip

# 3) manifest / smoke / doctor / rc
python scripts\release_manifest.py --zip dist\LabSentinel-portable-<日期>.zip
python scripts\release_manifest.py --verify dist
python scripts\smoke_portable.py --zip dist\LabSentinel-portable-<日期>.zip
python dist\portable\LabSentinel\scripts\doctor.py --root dist\portable\LabSentinel
python scripts\rc_regression.py --fast
```

## 验收证据

- 新 zip 名/大小/条目/sha256；含本批修复（抽查 `genai.py`/`plugins.py`/版本）；
- pytest（预期 ≥491+新增）、`npm test`、build gzip；
- manifest --verify OK；smoke 8/8；doctor FAIL 0；rc 4/4；release_check 全 PASS；
- app 内 `__pycache__` 条目=0；端口/进程/临时目录零残留；主库未动；
- 更新 `docs/发布核对报告.md`、`docs/release-notes-v0.1.0-alpha.md` 的数字（以 manifest 为准）。

## 约束

- 不发布/不推 tag/不 GitHub Release；不跑 24h/训练/4 路压测；
- 若 131–136 有 BLOCKED，包只含已完成提交，报告中列明；
- 发布硬阻塞（真实 24h/止损线/CI 实跑/签名）未解除，不得写“生产就绪”。
