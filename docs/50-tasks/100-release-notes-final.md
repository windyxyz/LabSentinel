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

## 执行报告

**flash 2026-11-04 完成（release_check 全 PASS 319 链接；数字全部来自 104 实跑 manifest/rc）**

### 数字对照表（文档值 vs manifest 实测值）

| 项 | 文档引用 | manifest/实测 |
|---|---|---|
| zip | `LabSentinel-portable-20261104.zip` | 同 |
| 大小 | 279,397,565 字节（279.4MB） | 同（release_manifest 实跑） |
| SHA256 | `4de5f25648156a514f56059f18bfaf1b7519b2ef1de0c61ef622128ac89b65d7` | 同 |
| 条目 | 12,605 | 同 |
| pytest | 491 passed | rc_regression 实跑 |
| 前端 gzip | 60.11KB | npm build 实跑 |
| smoke | 8/8（110 新语义：入口退出+残留收尾） | 实跑 |
| 包内 doctor | 10/0/0 | 解压根实测 |

### 改动文档

- release-notes：Features 补 Windows 收尾批次（109-126 全列，含"默认关闭"限定）+
  Known limitations 补**未验证声明**（24h/止损线、opt-in 三件套）；
- 发布公告「状态与边界」：补批次落地说明 + **发布候选非生产就绪**声明；
- 发布核对报告：追加「2026-11-04 最终 RC 核对」小节（完整数字表+阻塞清单勾选状态）；
- 未出现"已验证/生产就绪"表述（含英文 notes）。

### 遗留

- 无。测试数字 491 引用 104 的 rc_regression 实跑；文档零手抄未验证数字。
