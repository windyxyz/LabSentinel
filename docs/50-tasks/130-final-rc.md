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

## 执行报告（批次 130-139 收口，2026-11-04 flash）

**flash 2026-11-04 完成（pytest 504 passed / zip 279.8MB sha a8f8c75d… / smoke 8/8 / 包内 doctor 10/0/0 / rc 4/4 / release_check 全 PASS / vitest 26/26）**

- **含本批 131-136 全部**：插件 UI/AI 徽标+筛选/vitest 基线（26 用例）/安全审计修复（照片
  10MB、导入规模上限——后者挖出 cap 位置真缺陷）/View Transitions/事件时间线；
  另含主线复审 4 修复（genai 中文路径/插件入口/MQTT LWT/版本 alpha）；
- manifest：279,802,837B / 12,617 条目 / --verify OK / 缺可选仅卸载.bat（预期）；
- OpenAPI 快照重生成（86 paths/105KB，--check 一致；132/134/136 三次加端点后均需更新——
  教训：**改 API 必同步快照**）；
- vitest 26/26（CI 已加 npm test）；137/138/139 backlog 不变；
- **不发布/不推 tag**；阻塞不变：真实 24h、模型止损线、CI 实跑、签名。
- 清理：smoke/doctor 临时目录已删、端口无监听、主库未动。

### 主线条目清理与最终重建（2026-11-04）

- 已含主线 3 处小修（人脸上传先限流再读、时间线按小时精确筛选、View Transition 重名），
  另删除旧 demo 打包/健康检查/旧压测截图脚本、Docker 部署文件与文档、旧 zip/日志/缓存；
- **最终包**：`LabSentinel-portable-20261104.zip` **279,802,837B / 12,617 条目 /
  sha256 `a8f8c75d055fda0c08832c23b1de996e7992dad3510c90a615c999a76c2c2af4`**；
- 重跑：pytest **504 passed**、vitest **26**、build **63.22KB**、smoke **8/8**、
  包内 doctor **10/0/0**、rc **4/4**、release_check 全 PASS；app 内 pyc=0。
- 发布阻塞不变：真实 24h、模型止损线、CI 实跑、签名；**不发布/不推 tag**。
