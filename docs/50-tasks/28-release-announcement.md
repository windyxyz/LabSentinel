# 任务 28 · 发布公告与 Release 文案草稿（先读本目录 README.md）

> 依据：docs/70-release-plan.md 发布渠道。v0.1.0-alpha 发布时直接可用。

## 目标（纯文档）

1. `docs/发布公告.md`（中文，面向 V2EX/知乎/少数派等）：
   - 标题（3 个候选，突出"开源/本地/隐私"）
   - 一句话 + 三段式正文：痛点（人工盯屏不可持续、商业方案贵且上云）→ 做了什么（RTSP 接入、白大褂检测、人数统计、多渠道告警、WebRTC、移动端路线）→ 可信度证据（实测数据：CPU/GPU/NPU 三档 FPS、v2 模型逐类 AP、串行锁事故复盘、合规设计）
   - 部署一句话（Docker/Windows）+ 文档链接占位
   - **禁止夸大**：不得写"准确率 98%"等未经测试集验证的数字；引用数字必须来自 docs/52-model-eval.md 或 roadmap 实测记录
2. `docs/release-notes-v0.1.0-alpha.md`（英文，GitHub Release 用）：
   - Highlights / Features / Models & data（不入库声明）/ Security & compliance / Known limitations（真机触控未验、H264 兼容、v2 模型域差距）/ Installation / Links
3. 检查 README 顶部是否可加"Release 状态徽章"的占位说明（只写文档，不改 README——README 归任务 20 已改版，如需改动在其他任务）

## 约束

- 数字纪律：每个数字来自仓库内文档，附来源文件路径；不确定的写"计划中"
- 合规措辞：人脸功能"默认关闭"必须出现；不暗示医疗/安防认证
- 不发布（仅草稿）；不改代码

## 验收标准

两份文档语言自然、可直接发布；所有数字可溯源；合规声明齐全。

## 执行报告

### 交付物
- `docs/发布公告.md`（中文）：3 个候选标题（均突出开源/本地/隐私）；一句话定位；三段式正文
  （痛点→做了什么→可信度）；Docker/Windows 部署一句话；**"状态与边界"小节**明确 alpha 定位与不夸大声明
- `docs/release-notes-v0.1.0-alpha.md`（英文，GitHub Release 用）：Highlights / Features /
  Models & data（不入库声明）/ Security & compliance / Known limitations（真机未验、H264 兼容、
  域差距、单用户鉴权、无 CI 模型测试）/ Installation / Links 占位

### 数字溯源对照（验收要求：每个数字来自仓库文档）
| 数字 | 来源 |
|---|---|
| CPU 32–39 FPS / DirectML 64 FPS / NPU XINT8 40 FPS | docs/40-roadmap.md M0 退出条件表（本次 grep 复核原文在档） |
| ByteTrack 4 人 4 事件 vs 简易跟踪 15 条重复 | docs/40-roadmap.md M1 实施记录 |
| 服务端 74 项单元测试 | pytest 实跑（本会话多次复核）+ 50-tasks 各执行报告 |
| WebRTC 约 1 秒延迟 | 明确标注"待真实摄像头验证"，未写成实测结论 |
| v2 模型（11 类/11420 图） | docs/40-roadmap.md 数据扩充段；逐类 AP 数字**未引用**（训练中未出 test 评估） |

### 合规措辞自查
- "人脸比对模块**默认关闭**"在中英文两稿均出现 ✓
- 无"准确率 98%"类未验证数字；无医疗/安防认证暗示（另加"未取得任何认证，不替代人工管理制度"）✓
- 英文稿 Known limitations 含真实短板（域差距/单用户鉴权/H264 兼容）✓

### 遗留问题
- README 徽章区已存在（任务 20），发布时在徽章行追加一枚 release 徽章（一行改动，归 README 维护者）；
  按任务约束本次未改 README
- 发布公告的文档链接为仓库相对路径，GitHub 化后替换为实际地址
- v2 逐类 AP 引用位已留好（52-model-eval.md 训后自动追加，届时文案数字区可补充）
