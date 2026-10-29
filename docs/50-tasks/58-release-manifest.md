# 任务 58 · 发布产物清单与校验和（release manifest）

> 背景：便携包已能构建（任务 44/55），但发布时缺少**可复核的产物清单**——版本、大小、SHA256、
> 必需文件是否齐全、构建时的提交号。现在靠人工 `dir` + 手抄数字，容易漏文件或贴错数字；CI（任务 52）
> 也只生成 `SHA256SUMS.txt`。本任务把"发布前检查"脚本化，产出机器可校验的 manifest。

## 目标

1. **新增 `scripts/release_manifest.py`**（只用标准库）：
   - `--zip <path>`：默认取 `dist/LabSentinel-portable-*.zip` 中 mtime 最新者；`--out <dir>` 默认 `dist/`。
   - **必需条目检查**（zip 内路径，允许一层 `LabSentinel/` 前缀）：
     `runtime/pythonw.exe`、`app/server/app/main.py`、`app/server/.env.template`、
     `web/dist/index.html`、`scripts/portable_launcher.py`、`scripts/seed_demo.py`、
     `启动.bat`、`停止.bat`、`安装.bat`、`卸载.bat`、`media/*.mp4`、`m0-spike/*.onnx`（或包内模型目录，按
     `build_portable.py` 实际布局核对后写死）。
   - **可选条目**：`LabSentinel.exe`（无 csc 的构建会降级，缺失只警告不失败）。
   - **输出**：
     - `SHA256SUMS.txt`：`<小写 sha256>  <zip 文件名>`（与任务 52 的 pwsh 格式一致，便于 CI 复用）；
     - `RELEASE-MANIFEST.json`：`version`（优先包内 `app/VERSION`，无则 `server/app/api.py` 的
       `0.1.0-m1`，再退 `0.1.0-alpha`）、`zip`、`size_bytes`、`sha256`、`entries`、`missing_required`、
       `missing_optional`、`zip_entry_count`、`git_commit`（`git rev-parse --short HEAD`，失败写 null）、
       `git_dirty`、`generated_at`。
   - `--verify <manifest|dir>`：重算 zip 的 sha256 与大小并对比，失败 exit 2；成功打印 `OK`。
   - 必需条目缺失 → exit 2；可选缺失 → 警告 exit 0。
2. **文档**：
   - `docs/59-portable-install.md` 增加"发布产物清单 / 校验和验证"小节（含 `--verify` 用法）；
   - `docs/发布核对报告.md` 追加「2026-10-30 便携包发布核对」：**只写脚本实际输出**的数字
     （zip 名/大小/sha256/条目数/缺失项），不手抄旧数字、不编造。
3. **测试** `server/tests/test_release_manifest.py`：
   - 构造临时 zip（含最小必需条目 + 一个可选缺失）→ 生成 manifest/SHA256SUMS → 字段齐全、哈希正确；
   - 缺必需条目 → 返回码 2 且 `missing_required` 有内容；
   - `--verify` 对篡改后的 zip 失败、对原 zip 成功；
   - 测试不依赖真实 `dist/` 大包。

## 约束

- **只读现有 zip**，不重建、不解压全包；不改 `build_portable.py`（如确需一行集成调用，先在报告说明理由）。
- 不写 `dist/` 以外的文件；测试用临时目录。
- 不编造版本号/体积；文档数字必须来自本次脚本输出。
- Windows 路径 + 中文文件名（`启动.bat`）必须正确处理（zip 内 UTF-8）。

## 验收标准

- 对 `dist/LabSentinel-portable-20261029.zip` 实跑：manifest 完整、`--verify` 通过、退出码 0；
- 临时 zip 的正/反用例测试全绿；`pytest tests/ -q` 基线不下降（当前 91 passed）；
- 文档中的数字与脚本输出逐项一致。

## 执行报告

**flash 2026-10-30 完成（真实 zip 实跑通过，pytest 95 passed）**

- **改动文件**：新增 `scripts/release_manifest.py`（纯标准库）、`server/tests/test_release_manifest.py`
  （4 用例）；docs/59"CI 产物/校验和验证"小节扩为"发布产物清单 / 校验和验证"（含 release_manifest
  用法与 --verify）；docs/发布核对报告.md 追加「2026-10-30 便携包发布核对」（数字全部来自本次脚本输出）。
  未改 build_portable.py（无需集成——manifest 独立于构建运行）。
- **实跑输出**（dist/LabSentinel-portable-20261029.zip）：version=2026.10、277,673,200 字节、
  sha256=6f61a730…c734、条目 12,544、必需条目齐全、可选仅缺 `卸载.bat`（安装器生成属预期）、
  git=d17dd59 dirty=True、`--verify` OK、退出码 0。输出 dist/SHA256SUMS.txt + RELEASE-MANIFEST.json。
- **布局核对说明**：必需条目按 build_portable 实际布局写死（`app/web/index.html` 而非规格草稿的
  `web/dist/index.html`；模型在 `models/`；`卸载.bat` 归可选——zip 内本就没有）。
- **测试**：4 用例（全条目 manifest 字段齐全+哈希正确 / 缺必需 rc=2 且 missing_required 有内容 /
  verify 原包 OK+篡改 rc=2 / 最新 zip 选取）。全量 **pytest 95 passed**（基线 91 + 新 4）。
- **修的两个小坑**：①归一化名与包内原名混用导致读 VERSION KeyError（加 orig 映射）；
  ②verify 约定 manifest 与 zip 同目录（测试已按此约定放置）。
- **遗留**：无。CI（任务 52）的 pwsh Get-FileHash 产物格式与 SHA256SUMS.txt 一致，可无缝替换为
  本脚本以同时获得 JSON manifest（记为 CI 启用时的小优化）。
