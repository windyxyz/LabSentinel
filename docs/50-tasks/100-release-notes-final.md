# 任务 100 · 发布文案与核对报告终稿（P2，纯文档）

> 背景：57–104 完成后，发布公告/Release notes/核对报告里的功能、体积、测试数字会过时；必须用
> 最终实测数字统一，并明确“尚未 24h 验证”。本任务只改文档，不发布。

## 目标

在 75（新 zip/manifest）与 97（RC 回归）完成后执行：

1. `docs/release-notes-v0.1.0-alpha.md`：
   - 功能列表按实际完成项更新（57–104 中已完成者，未完成不写）；
   - 数字：pytest N passed、前端 gzip 体积、便携包 zip 大小/SHA256（引用 `RELEASE-MANIFEST.json`，
     不手抄）；
   - 明确“真实 RTSP 摄像头 24h 尚未验证 / 模型止损线未达标”；
   - 不出现“已验证/生产就绪”类表述。
2. `docs/发布公告.md`：同上，三段式（是什么/能做什么/边界与限制）；标题候选保留。
3. `docs/发布核对报告.md`：
   - 追加「2026-10-30/31 最终核对」：自动项（rc_regression 表格）、便携包 manifest、已知阻塞；
   - 发布清单勾选状态：真实 24h、模型止损线、干净 VM、GitHub Actions、zip+sha、签名；
   - 明确“当前可发布候选 = 待 24h 通过后才可对外”。
4. `README.md` 功能/快速开始如需更新（只改与新功能相关的事实）。
5. `docs/35-api-reference.md` 端点索引与实现一致（可跑 release_check 辅助）。

## 约束

- 只改文档；不发布/不推 tag/不建 Release；
- 所有数字来自本次 `rc_regression`/manifest/实际命令输出，禁止沿用旧数字；
- 不夸大模型指标（Lab Coat P 49% / No Lab Coat P 77% 等引用 docs/52）；
- 不删除历史核对记录，只追加新小节。

## 验收

- `python scripts/release_check.py` FAIL 0（链接/版本/敏感信息/必需文件）；
- 文档中的 zip 名/大小/sha 与 `dist/RELEASE-MANIFEST.json` 一致；
- 报告附一致性核对表（文档值 vs manifest 值）。

## 执行报告

（待填：改动文档 / 数字对照 / release_check / 遗留）
