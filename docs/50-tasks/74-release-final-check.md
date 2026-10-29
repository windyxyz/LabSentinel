# 任务 74 · 发布文档终检 + 仓库一致性检查

> 背景：发布前需要确认"文档说的和仓库实际一致"：相对链接不断、版本号统一、没有密钥/TODO 残留、
> 必需文件齐全、发布公告/Release notes 的数字与实际 zip 一致。人工核对易漏，本任务脚本化并出终检报告。

## 目标

### 1. 新增 `scripts/release_check.py`（只用标准库）

```
python scripts/release_check.py [--root .] [--zip dist/LabSentinel-portable-*.zip] [--json]
```

检查项（每项 PASS/WARN/FAIL + 证据）：

1. **相对链接**：扫描 `README.md`、`docs/*.md`、`docs/50-tasks/*.md` 中的
   `[...](相对路径)` 与 `![](...)`，目标文件存在；跳过 http(s)/锚点/代码块内链接；
2. **版本一致性**：`server/app/main.py` 的 FastAPI version、`server/app/api.py` `/api/system`
   返回的 version、`docs/release-notes-v0.1.0-alpha.md`/`docs/发布公告.md` 中出现的版本串，
   列出不一致（允许 release notes 使用 `v0.1.0-alpha`）；
3. **敏感信息**：仓库文本文件（排除 `.git`、`dist`、`m0-spike`、二进制）grep 常见密钥模式
   （`sk-`、`AKIA`、`ghp_`、`-----BEGIN .*PRIVATE KEY`、`LS_ADMIN_TOKEN=` 非空等）；
   测试夹具/示例中的假值需在 allowlist 说明；
4. **TODO/FIXME**：统计源码与文档中的 TODO/FIXME（不要求为 0，列出清单供判断）；
5. **必需文件**：LICENSE、NOTICE、README、`.env.example`、`server/requirements.txt`、
   `web/package.json`、`docs/55-handoff.md`、`docs/运维手册.md`（73 完成后）；
6. **便携包**（可选 `--zip`）：复用任务 58 的 manifest/校验（调用其函数或读取
   `RELEASE-MANIFEST.json`），确认 zip 存在、sha256 与 manifest 一致、必需条目齐全。

- `--json` 机器可读；退出码：有 FAIL → 2，仅 WARN → 0。
- 不修改任何文件（纯检查）。

### 2. 测试 `server/tests/test_release_check.py`

- 临时目录造 md 文件：有效链接 PASS、断链 FAIL、http/锚点忽略；
- 版本不一致 → WARN/FAIL（定义清楚）；
- 假密钥模式 → FAIL；allowlist 生效；
- 必需文件缺失 → FAIL；
- 测试用临时目录，不扫真实仓库（避免 CI 抖动）。

### 3. 文档终检

- 对真实仓库运行 `release_check.py`，修复所有 FAIL（断链/真密钥/缺文件）；WARN 逐条判断并记录；
- 更新 `docs/发布核对报告.md`：追加「2026-10-30/31 发布终检」小节，写脚本输出摘要、
  仍存在的阻塞（真实摄像头 24h/模型止损线/GitHub 远端/签名）；
- 更新 `docs/release-notes-v0.1.0-alpha.md` 与 `docs/发布公告.md`：便携包版本/大小/SHA256
  与任务 58 输出一致（不重复手抄，引用 manifest）；**不得宣称已通过真实摄像头 24h**；
- `README.md` 文档索引加入运维手册与发布核对报告。

### 4. 发布清单（人工项）

在 `docs/发布核对报告.md` 列出最终发布步骤（不是自动执行）：
- [ ] 真实 RTSP 摄像头 24h 通过（用户侧）
- [ ] 模型止损线达标（Lab Coat ≥60%、误报 ≤20%/天）
- [ ] 干净 Windows VM 安装/启动/卸载/自启
- [ ] GitHub 远端 + Actions 实际跑通（52）
- [ ] zip + SHA256SUMS 上传 Release；公告/notes 定稿
- [ ] 可选代码签名

## 约束

- 不自动修改文档内容（修复由执行者手工做，脚本只报告）；
- 不联网检查 http(s) 链接（只检查本地相对链接，避免夜晚网络不稳定）；
- 不扫描 `m0-spike/`、`dist/`、`node_modules/`、`.git/` 的大文件；跳过二进制；
- 不发布、不推送、不打 tag。

## 验收标准

- `pytest tests/ -q` 全绿（基线 91 + 新增）；
- 真实仓库运行 `release_check.py`：FAIL 0（或全部说明原因），报告附摘要；
- 文档数字与 `RELEASE-MANIFEST.json` 一致；公告不含"已验证 24h"类表述；
- 发布清单可勾选、无遗漏硬阻塞。

## 执行报告

（待填：改动文件 / 检查摘要 / 修复项 / 遗留阻塞）
