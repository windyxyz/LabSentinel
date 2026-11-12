# ADR-0011：Rust-first 全面重构战略

- **状态**：Accepted（2026-11-13，维护者决策：「推进整个项目全面 Rust 重构（实在无法
  重构实现的功能除外）；后续新开发一律 Rust 优先——直接写 Rust，而不是先写 Python
  再迁移」；执行授权：「Rust 设计和自主开发就行」）
- **背景**：0.2.0 时点（边界详见 `docs/dev/架构设计.md`）——数据面 ls-analyzer
  sidecar 已完成并灰度（默认 `analyzer_default="python"`）；控制面 ls-server 路由
  对齐 105/106 但独立模式缺分析 worker（start_camera/scan 503），不在任何生产启动链；
  产品唯一可用形态仍是 Python（FastAPI + CameraWorker 线程 + 嵌入式运行时分发）。
  原 N+1 灰度节奏按「版本号缓慢翻转」规划；本 ADR 把终点改为**完全重构**并授权
  自主推进。

## 决策

1. **终点**：Rust 单二进制（L4）成为产品形态——ls-server 承载全部控制面 + 分析
   worker + 存证 + 告警，嵌入式 Python 运行时从分发物中退役。
2. **Rust-first 规则**：新功能一律直接写 Rust；Python 侧只做两件事——
   过渡期缺陷修复（P1/P2 级）与行为契约参照，不再新增功能。
3. **豁免清单（确实无法重构，逐项确认后固化）**：
   - `tools/` 训练与数据飞轮生态（ultralytics/torch——Rust 无等价训练栈，且
     `tools/README-ENGINE-BOUNDARY.md` 已宣告其长期 Python 属性）；
   - `deploy/windows/installer.iss`（Inno Setup 脚本，非进程代码）；
   - OpenVINO 推理后端（Rust 无官方绑定；CPU 用 tract、GPU 用 ort load-dynamic
     的 DML/CUDA 替代，OpenVINO 随 L4 退役并在文档标注）。
   - 待评估：Windows 启动 UX（托盘/双击体验）在 L4 后用 Rust 原生实现还是保留
     薄壳——RA4 阶段决定。
4. **灰度机制保留**：`analyzer_default` 开关与 Python 回退路径在过渡期不动；
   翻转条件从「N+1 版本」改为「RA1 验收通过」。
5. **行为契约单一事实源**：`docs/openapi.json` 快照 + `proto/analyzer.proto` +
   Python CameraWorker 生命周期语义；Rust 侧对齐以黄金语料向量为准
   （ls-infer/ls-track 的 vectors.rs 契约测试不回退）。

## 分阶段路线（验收标准见 docs/plan/rust-first-roadmap-2026-11-13.md）

| 阶段 | 内容 | 退出条件 |
|---|---|---|
| RA1 | ls-server 分析 worker：进程内复用 ls-ingest/ls-infer/ls-track 管线，事件直写 ls-store，告警走 ls-dispatch；folders/scan 一并实现 | start_camera/scan 摘掉 503；黄金语料一致；路由 106/106 |
| RA2 | 控制面补全：face/ppe/genai/plugins 真实现、MQTT bridge（rumqttc 在 ls-dispatch）、模型管理 | 503 占位清零（豁免项除外） |
| RA3 | L4 单二进制：ls-server 与 analyzer 管线合并进程，build_rust.py 转正式产物；便携包 = 单 exe + 模型 + web 静态 | smoke_portable Rust 形态 8/8 |
| RA4 | 部署/启动链替换评估：install_portable/portable_launcher 逐项 Rust 化或被单二进制取代；Python 退役清单 | 分发物零嵌入式 Python |

## 风险与对策

- **GPU 推理**：ls-infer 的 ort 为 load-dynamic（免 libclang 编译），DML 实测
  69.43fps 单路已验证数据面等价；CUDA 待 Windows 真机验证，失败则该后端豁免。
- **回归面**：Python 契约测试 626 例在过渡期继续跑（防 Python 侧修复引入回归）；
  Rust 侧以 routes_guard + 黄金向量 + --selftest-db 三件套护住。
- **发布节奏**：v0.2.0 发布暂缓中（用户指示）；恢复时 tag 重打即可，RA 里程碑
  不阻塞 0.2.x 补丁发布。

## 关联

ADR-0002（server-side analysis）、ADR-0008（跨语言 schema 兼容）、
ADR-0010（安装器双形态——Python 安装器能力在 RA4 前保留）。
