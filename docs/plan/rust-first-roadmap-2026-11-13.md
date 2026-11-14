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
- [x] RA1.2 worker 生命周期：start_camera 摘 503（WorkerConfig 从 DB 组装
      cameras/zones/rules/calendar、退避重连 1/5/30/60s + 存活>5s 重置 + 超限
      given_up→error 对齐 Python CameraManager、detector 参数化 DetectorConfig
      进 lib、DELETE /cameras/:id 补注册、bulk/snapshot 接 worker）——
      ls-server 33 例 + workspace 105 passed + clippy/fmt 绿
- [x] RA1.3 告警链路：规则命中 → ls-dispatch 投递（8 渠道）——
      `Jobs::dispatch_event` 复用 escalation 链路组件（静默/过滤/限流/投递行），
      EventSink 落地后首发派发；WS 广播与外部投递解耦
- [x] RA1.4 folders/scan 实现（wake-or-start + folder worker 扫描登记，
      识别推理 RA2——图像解码依赖引入后补）
- [x] RA1.5 门禁：路由 **106/106 = 100%**（folders/scan 摘牌，coverage 实测
      未实现 0 条）+ ls-server 37 例 + workspace 109 passed + clippy/fmt 绿
- 验收：独立 Rust 形态代码链路齐备（RTSP 帧循环→事件→存证→告警；folder
  扫描→登记）；**端到端真机验收（真实 RTSP/视频挂机）留 RA1 验收项**，
  与 24h 挂机人工事项合并执行。

## RA2：控制面补全（2026-11-13 完成，S1~S9 全部落地）

- [x] face/ppe 管线：PPE runner 参数化进 lib（`ppe.rs`）+ worker/folder 注入；
      face CompreFace 控制面（合规门/consent 强制/同意表；1:N 识别数据面豁免在
      Python/sidecar）；模型管理走 env（LS_MODEL_PATH/LS_PPE_MODEL），assets.lock
      哈希校验在 ppe/download 已有
- [x] genai 真实现（双 provider/严格 JSON/回写不改 status/回环红线/status 九字段）
- [x] plugins 机制对齐（manifest 严格校验/state.json/启停/日志 tail/坏清单 error 项）
- [x] MQTT bridge 接线（默认关零行为变化；payload v1 只加不减）
- [x] folder 识别管线（ffmpeg 单图解码→YOLO→区域/PPE→违规快照/事件/event_id 回填）
- [x] /api/system 契约补齐（today_counts/unhandled_today/alerts）+ model_status
      形状对齐 + update/check 拉清单真实现（minisign 如实豁免）
- [x] 验收：ls-server **503 占位清零**（豁免项：update 签名验证、face 1:N 识别
      数据面、OpenVINO——均如实标注）；workspace 115 passed + clippy/fmt 绿

## RA3：L4 单二进制（2026-11-13 完成）

- [x] ls-server 进程内嵌分析管线：RA1.2 的 worker.rs 即进程内 ffmpeg 帧循环
      （从不 spawn sidecar——"去 gRPC 自环"由方案 C→A 天然达成）
- [x] build_rust.py 转正式产物：release workflow nightly-rust job 重写为
      web build → build_rust --zip → **RA3 冒烟** → artifact + `gh release upload`
      （保持独立 job，Rust 失败不阻塞便携主链）
- [x] 便携包 Rust 形态：labsentinel-server.exe + web/dist + models + go2rtc +
      .env.example + README（**30.8MB vs Python 便携包 198MB**——嵌入式
      CPython runtime 退役的价值实证）；冒烟 `scripts/smoke_rust.py`
      （解包/就绪/backend=rust/前端 SPA 4/4 PASS，本机实测）
- [x] web_dist 缓存策略：0.2.0 已接线（middleware static_cache_policy：
      assets/* immutable 1 年 + SPA 回退 no-cache），冒烟抽验资产 200 通过

## RA4：部署/启动链替换评估（2026-11-13 评估完成；2026-11-14 过渡期收口完成）

逐项判定（install_portable / portable_launcher 能力矩阵）：

| 能力 | 判定 | 依据 |
|---|---|---|
| 复制 staging→安装目录 | **Inno 消解** | [Files] 原生 |
| 桌面/开始菜单快捷方式 | **Inno 消解** | [Icons] 原生 |
| HKCU 卸载注册表 | **Inno 消解** | AppId 键原生 |
| 开机自启 | **Inno 消解** | [Icons] {userstartup} |
| 覆盖升级 | **Inno 消解** | UsePreviousAppDir |
| 单实例锁/端口试探/开浏览器 | **Rust 重写 ✅ 已落地** | `--launch`（端口试探防双实例 + launcher.lock "pid port" 契约对齐 + 就绪开浏览器） |
| finalize 健康检查（health_check.py） | **Rust 重写 ✅ 已落地** | `--selftest-http`（临时端口真启动轮询 ready ≤60s，本机实测 exit=0） |
| 升级版本归档/回滚（versions/） | **Rust 重写 ✅ 已落地** | `--archive/--rollback`（排除 data/versions/缓存、版本名防穿越、current.txt 选次新归档；4 单测 + 真二进制冒烟） |
| deploy.py（源码模式） | **豁免** | 开发工具，长期 Python |
| tools/ 训练工具链 | **豁免** | ADR-0011 豁免清单 |

- [x] portable_launcher 消解件：`labsentinel-server.exe --launch`（端口试探 +
      单实例锁 + 开浏览器；托盘留 RA4.2 可选）
- [x] health_check 消解件：`labsentinel-server.exe --selftest-http`（本机实测
      ready=true exit=0；Inno finalize 后续改调此子命令——Python 安装器保留期
      内两条路径并存）
- [x] 分发物零嵌入式 Python 验证：Rust 形态 zip 30.8MB 本就不含 runtime
      （RA3 冒烟覆盖）；Python 便携包/installer 按 ADR-0010/RA4.2 保留
- [x] RA4.2（过渡期末，2026-11-14 完成）：升级归档/回滚 Rust 化；Inno 安装器随
      Python 分发物退役（发行形态改为 zip + `LabSentinel.bat`）；`server/` 与 Python
      便携打包/安装/运维脚本全部删除；release.yml/tests.yml 改为 Rust 单产物门禁
- [x] 过渡期 Python 纪律：P1/P2 必修（已全部闭环），P3 仅台账，不再大重构

> **收口记录（2026-11-14）**：RA1–RA5 全部完成；退役前全量门禁 Python 631 passed /
> Rust 122 / Web 44 / acceptance_rust_only 39/39 / acceptance_vlm 9/9 / smoke_rust PASS。
> 详见 ADR-0011「执行结果」与 `docs/plan/status.md` 第五轮。

## 过渡期纪律（历史，已随 2026-11-14 收口结束）

- Python 侧：只修 bug 不加功能；契约测试 626 例持续全绿（退役前 631 passed / 7 skipped）。
- Rust 侧：routes_guard（inventory 同步）+ 黄金向量 + `--selftest-db` 三护住。
- 灰度开关 `analyzer_default` 翻转点 = RA1 验收（替代原 N+1 版本节奏）。
