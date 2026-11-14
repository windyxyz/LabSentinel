# ADR-0011：Rust-first 全面重构战略

- **状态**：Accepted（2026-11-13，维护者决策：「推进整个项目全面 Rust 重构（实在无法
  重构实现的功能除外）；后续新开发一律 Rust 优先——直接写 Rust，而不是先写 Python
  再迁移」；执行授权：「Rust 设计和自主开发就行」）
  **过渡期结束（2026-11-14）**：RA1–RA4 + RA5（模型模块化）全部完成，Python 运行时
  （`server/`）与 Python 便携打包链已退役；`labsentinel-rust-*.zip` 单二进制为唯一
  发布形态，详见文末「执行结果」。
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
     `tools/README-ENGINE-BOUNDARY.md` 已宣告其长期 Python 属性）；`scripts/` 下的
     构建/验收支具同理（Python 仅作开发工具，不进分发）；
   - OpenVINO 推理后端（Rust 无官方绑定；CPU 用 tract、GPU 用 ort load-dynamic
     的 DML/CUDA 替代，OpenVINO 随 L4 退役并在文档标注）；
   - ~~`deploy/windows/installer.iss`~~：Inno Setup 安装器随过渡期结束退役，
     发行形态改为“解压即用 zip + LabSentinel.bat”（版本归档/回滚由
     `labsentinel-server --archive/--rollback` 承担），见执行结果。
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
ADR-0010（安装器双形态——随过渡期结束退役，zip 单包 + --archive/--rollback 取代）。

## 执行结果（2026-11-14，过渡期收口）

| 阶段 | 结果 | 关键证据 |
|---|---|---|
| RA1 | ✅ | 进程内分析 worker（相机/文件夹）、首发 8 渠道、folders/scan 503 清零；路由 106/106 |
| RA2 | ✅ | face/ppe/genai/plugins/MQTT/模型管理真实现；503 占位清零（USB 枚举豁免） |
| RA3 | ✅ | `labsentinel-rust-*.zip` 单二进制（含模型/ffmpeg/onnxruntime/启动器），smoke_rust 4/4 |
| RA4 | ✅ | `--archive/--rollback` Rust 化、`prepare_rust_assets.py` + `build_rust --strict`、数据目录/版本自举；分发物零嵌入式 Python |
| RA5 | ✅ | VLM 三 provider + 按摄像头插拔 + 文件夹/视频通路；mock 验收 9/9 |

退役前全量门禁（2026-11-14）：Python 631 passed / 7 skipped；Rust workspace 122；
Web 44 + build；`acceptance_rust_only` 39/39；`acceptance_vlm` 9/9；`smoke_rust` PASS。
退役动作：删除 `server/` 与 Python 便携打包/安装/运维脚本、Inno 安装器；
`tests.yml` 改为 Rust + Web 门禁，`release.yml` 仅发布 Rust zip + `SHA256SUMS-rust.txt`。
保留：`tools/` 训练工具链、`scripts/` 构建/验收支具（Python 仅作开发工具）。
