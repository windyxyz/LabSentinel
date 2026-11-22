# Progress — LabSentinel 会话进度与交接快照

## Latest Handoff Snapshot

- **Task-ID**: `LS-050-057`（`DONE`）
- **Task-Name**: LabSentinel v0.5.0 补齐与本机发布验收
- **基线**: `main @ 3ebda37`，启动时有 28 个未提交文件，含 v0.5.0 版本号和前次 Rust 修补。已在这些修改上继续。
- **本轮实现**:
  - LS-050 / LS-051：复验 Motion Gate、PPE 滑块、单人留守与 VLM 提示词。修复签名缩略图访问、JPEG 存证输入、go2rtc 参数和内存流同步、WS 鉴权与检测元数据、值守快照回退。
  - LS-052 / LS-053：已有安全区顶点可撤销与重做，保留可视化排班与事件快审。告警重试尊重退避并可立即消费，局域网配置与自重启生效，Windows 自启写入 HKCU Run 的命名值。
  - LS-054：快捷标注、人员预标注、样本隔离与 YOLO ZIP、诊断脱敏、只读令牌隔离与吊销。
  - LS-055 / LS-056：读取 ONNX NCHW 尺寸、FP16 输入输出、进程级 ORT 环境、逐类和逐图 A/B 评测，以及公开数据源、词表映射、混合训练和 FP16 导出。
- **Windows 实测**:
  - `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check` 与 `cargo +stable-x86_64-pc-windows-gnu clippy --workspace --all-targets --all-features -- -D warnings` 均退出 0。
  - `cargo +stable-x86_64-pc-windows-gnu test --workspace --all-features --no-fail-fast` 通过 184 项，0 失败。显式使用仓库随包 FFmpeg 和 ONNX Runtime DLL。
  - `npx.cmd vitest run` 通过 15 个文件、93 项测试。`npm.cmd run build` 生成 61 个转换模块，492ms。
  - `python -X utf8 scripts/contract_audit.py` 通过。117 条前端 API 匹配后端 155 条清单项，安装脚本 BOM 正常。
  - `python -m unittest discover -s tools/tests -v` 通过 11 项。`python -m py_compile` 对 `tools/*.py` 通过。
  - 隔离本机实测中，FP32 和 FP16 模型同进程运行，FP16 评测返回 512×256 和 1 个检出。无令牌开启局域网返回 422，更换端口后的子进程按新读取保护配置就绪。
  - 隔离缩略图实测通过。无凭证访问返回 401；使用对应路径签名获得 320×180 JPEG，第二次请求命中缓存，签名用于其他路径时返回 401。
  - `python -X utf8 scripts/bump_version.py 0.5.0` 输出同步成功。
  - `python -X utf8 scripts/build_rust.py --strict --zip` 退出 0，生成 100.4 MB ZIP。
  - `python -X utf8 scripts/rust_route_coverage.py --exe <release exe> --fail-under 95` 输出 `106/106 = 100.0%`，已实现 154 个唯一方法和路径组合。
  - `python -X utf8 scripts/build_setup_exe.py` 调用 Inno Setup 7，`rc: 0`，生成 75.1 MB 安装器。
  - `python -X utf8 scripts/smoke_rust.py --zip Resources/dist/labsentinel-rust-0.5.0-20261122.zip --timeout 90` 通过 5/5。返回 `ready`，`backend=rust`，`version=0.5.0`，`ppe_enabled=True`，SPA 静态资源可达，`shutdown=200`，进程退出码为 0。
- **发布产物**:
  - `Resources/dist/labsentinel-rust-0.5.0-20261122.zip`，100426313 字节，SHA256 `92E1AB7E9F3795345FF49B5AD814F5253A3B0BDE88B4337B25CF95C6C37D3147`。
  - `Resources/dist/LabSentinel-setup-0.5.0.exe`，75119342 字节，SHA256 `B056462260602A2044B32E44E873EFD6F35722D20FCC26FAFD59A3B4D71D625D`。
- **现场覆盖边界**: 本轮运行了 CPU FP16 合成模型、随包 go2rtc 注册和注销、ZIP 本机冒烟。真实 RTSP 长跑、GPU/DirectML 专项压力、外部 VLM 和渠道交付、公开数据全量训练，以及安装器交互安装/升级/卸载需要相应的外部环境。Vitest 输出 Vite native loader 提示和 jsdom Canvas 提示，Vite 构建退出 0。
- **Next First Command**: `git log -1 --stat` 审阅本地 v0.5.0 提交。正式发行资产由 CI 生成和上传。

---

## LS-050-057 本轮改动文件清单

```text
CHANGELOG.md
README.md
VERSION
deploy/windows/installer.iss
docs/dev/data-flywheel.md
docs/guide/安装与卸载.md
docs/plan/status.md
engine-rs/Cargo.lock
engine-rs/Cargo.toml
engine-rs/crates/ls-analyzer/Cargo.toml
engine-rs/crates/ls-analyzer/src/evidence.rs
engine-rs/crates/ls-analyzer/src/motion.rs
engine-rs/crates/ls-analyzer/src/pipeline.rs
engine-rs/crates/ls-analyzer/src/ppe.rs
engine-rs/crates/ls-dispatch/Cargo.toml
engine-rs/crates/ls-dispatch/src/lib.rs
engine-rs/crates/ls-infer/Cargo.toml
engine-rs/crates/ls-infer/src/post.rs
engine-rs/crates/ls-infer/src/preprocess.rs
engine-rs/crates/ls-infer/src/yolo.rs
engine-rs/crates/ls-ingest/Cargo.toml
engine-rs/crates/ls-ingest/src/child_guard.rs
engine-rs/crates/ls-ingest/src/winjob.rs
engine-rs/crates/ls-jobs/Cargo.toml
engine-rs/crates/ls-jobs/src/lib.rs
engine-rs/crates/ls-jobs/tests/jobs.rs
engine-rs/crates/ls-server/Cargo.toml
engine-rs/crates/ls-server/src/detect.rs
engine-rs/crates/ls-server/src/envfile.rs
engine-rs/crates/ls-server/src/go2rtc.rs
engine-rs/crates/ls-server/src/handlers/cameras.rs
engine-rs/crates/ls-server/src/handlers/dataset.rs
engine-rs/crates/ls-server/src/handlers/events.rs
engine-rs/crates/ls-server/src/handlers/misc_status.rs
engine-rs/crates/ls-server/src/handlers/ops.rs
engine-rs/crates/ls-server/src/handlers/zones_rules.rs
engine-rs/crates/ls-server/src/main.rs
engine-rs/crates/ls-server/src/middleware.rs
engine-rs/crates/ls-server/src/router.rs
engine-rs/crates/ls-server/src/tests/routes_guard.rs
engine-rs/crates/ls-server/src/util.rs
engine-rs/crates/ls-server/src/worker.rs
engine-rs/crates/ls-store/Cargo.toml
engine-rs/crates/ls-store/src/lib.rs
findings.md
progress.md
scripts/build_rust.py
scripts/build_setup_exe.py
scripts/rust_route_coverage.py
scripts/smoke_rust.py
task_plan.md
task_registry.md
tools/README.md
tools/dataset_hub.py
tools/datasets_registry.json
tools/eval_report.py
tools/export_onnx.py
tools/merge_datasets.py
tools/tests/test_dataset_tools.py
tools/train_ppe.py
web/package-lock.json
web/package.json
web/src/App.vue
web/src/api.js
web/src/components/AnnotatorModal.vue
web/src/components/VideoOverlayCanvas.vue
web/src/components/WebrtcPlayer.vue
web/src/overlay.js
web/src/views/Events.vue
web/src/views/FolderImages.vue
web/src/views/Models.vue
web/src/views/Settings.vue
web/src/views/Wall.vue
web/src/views/Zones.vue
web/tests/api.test.js
web/tests/ls042_regressions.test.js
web/tests/wall_overlay.test.js
web/tests/zones.test.js
修改记录_会话备忘.md
```

## 会话日志

### 2026-11-22 · `LS-040` 全仓四线审阅与接力文件入库
- 完成 Rust 管线 / Rust 控制面与安全 / Vue 前端与契约 / CI 与打包安装器四线只读审阅。
- 识别 20+ 项 P0 级缺陷与 35+ 项 P1 级缺陷，产出 6 批次实施计划并获用户全量批准。
- 更新接力 7 件套并提交至 `main`，随即锁定 `LS-041` 开工。
