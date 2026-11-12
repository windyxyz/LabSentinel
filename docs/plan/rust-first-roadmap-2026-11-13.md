# Rust-first 全面重构路线图（ADR-0011 执行计划）

> 创建：2026-11-13。终点：Rust 单二进制成为产品形态（L4），确实无法重构的除外
> （豁免清单见 ADR-0011）。授权：维护者「Rust 设计和自主开发就行」。
> 每阶段一个或多个 commit，门禁全绿才推进；进度打勾记录在本文件。

## RA1：ls-server 分析 worker（进行中）

把「分析 worker」接进 Rust 控制面，替换 Python CameraWorker/sidecar 编排：

- [x] RA1.0 设计勘察（管线内幕/ls-server 接缝/proto 契约/方案 A/B/C 对比；
      结论 C→A：pipeline/evidence 拆 lib + 进程内 worker，弃 sidecar 自环）
- [x] RA1.1 事件出口改造（首切）：ls-analyzer 拆 lib；ls-store 补
      `set_event_video` + `rules_all` 补 `days_of_week`；ls-server `worker.rs`
      `EventSink`（去重 4096/截图/入库/WS 广播/finalize encode_mp4 回填）——
      workspace 99 passed + clippy/fmt 绿
- [ ] RA1.2 worker 生命周期：start_camera/stop_camera 真实现（配置读 DB、
      重试/配置版本重推/watchdog 对齐 Python 语义；WorkerConfig 参数化
      make_detector 的 env 依赖）
- [ ] RA1.3 告警链路：规则命中 → ls-dispatch 投递（8 渠道）+ escalation
- [ ] RA1.4 folders/scan 实现（ls-ingest folder 源已有，摘 503）
- [ ] RA1.5 门禁：路由 106/106（folders/scan 摘牌）+ 黄金语料向量一致 +
      ls-server 测试全绿 + clippy/fmt
- 验收：独立 Rust 形态可真实分析 RTSP/文件夹源并产出事件/存证/告警。

## RA2：控制面补全（待 RA1）

- [ ] face/ppe 管线真实现（模型管理走 ls-store + assets.lock 哈希）
- [ ] genai 占位实现（对齐 Python genai 提供者语义）
- [ ] plugins 机制对齐（manifest 加载/启用禁用）
- [ ] MQTT bridge（rumqttc 在 ls-dispatch 已有基础，T-329）
- [ ] 验收：ls-server 503 占位清零（OpenVINO 等豁免项除外，如实标注）。

## RA3：L4 单二进制（待 RA2）

- [ ] ls-server 进程内嵌分析管线（去 sidecar gRPC 自环，或保留为可选模式）
- [ ] build_rust.py 转正式产物（release workflow 接入）
- [ ] 便携包 Rust 形态：单 exe + models + web 静态 + .env；smoke 8/8
- [ ] web_dist 静态服务 + SPA 回退已就绪（0.2.0 已接线），补缓存策略对齐

## RA4：部署/启动链替换评估（待 RA3）

- [ ] install_portable 各能力逐项判定：Rust 重写 / 单二进制消解 / 豁免保留
- [ ] portable_launcher：单二进制下大概率消解为 exe 直启 + 托盘（待评估）
- [ ] health_check/升级归档/回滚语义在 Rust 安装器（或 Inno [Code]）中的等价物
- [ ] Python 退役清单 + 分发物零嵌入式 Python 验证
- [ ] 过渡期 Python 缺陷修复承诺：P1/P2 必修，P3 仅台账（不再大重构 Python）

## 过渡期纪律

- Python 侧：只修 bug 不加功能；契约测试 626 例持续全绿。
- Rust 侧：routes_guard（inventory 同步）+ 黄金向量 + `--selftest-db` 三护住。
- 灰度开关 `analyzer_default` 翻转点 = RA1 验收（替代原 N+1 版本节奏）。
