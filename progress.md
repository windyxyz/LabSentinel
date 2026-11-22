# Progress — LabSentinel 会话进度与交接快照

## Latest Handoff Snapshot

- **Task-ID / Task-Name**：`LS-061`（`ACTIVE`）/ 首页状态、Release 入口、报表文案与告警布局。
- **基线**：`main`，`LS-060` 本机验收已通过。GitHub 默认分支为 `main`，当前公开 Release 为 `v0.5.0`。
- **范围**：用户指示全部作为 `v0.6.0`，计划后直接实施，全部验收通过后发布 GitHub。任务 `LS-060`～`LS-068` 逐项推进。
- **Next First Command**：`rg -n 'checkUpdate|gpuHint|操作审计|class="card channel"' web/src`。

---

## Previous Handoff Snapshot · LS-060

- **实现**。资产锁定 go2rtc 1.9.15 和 MIT 许可，打包从干净资源目录取得网关。Rust 监管进程和注册列表，修正 FFmpeg RTSP 的 `-timeout`，按真实首帧切换播放。只读分享使用服务端可中断的 MSE 中继。首次升级按实际 `VERSION` 恢复旧版，网络设置写入数据目录，端口冲突保留监听，开发启动改用托管网关。
- **门禁**。Windows GNU `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`cargo test --workspace --all-features` 均退出 0，共 187 项 Rust 测试。`npx.cmd vitest run` 93 项通过，`npm.cmd run build` 退出 0。`python -X utf8 scripts/contract_audit.py` 为 117/155 全匹配且 Inno BOM 正常，`rust_route_coverage.py --fail-under 95` 为 106/106，`--fail-under 101` 按预期退出 1。
- **浏览器与部署实测**。`node web/scripts/smoke_webrtc.mjs <隔离 staging>` 用随包 FFmpeg、go2rtc、ORT 和实验服模型验证 RTSP 解码、WebRTC 首帧、网关故障降级和恢复、分享 MSE 解码与服务端吊销，吊销后的快照返回 401，管理员画面继续播放。基于 v0.5.0 官方 ZIP 的隔离往返验证旧 EXE、前端和数据保留，通过占用目标端口与有效端口重启复测。资产下载和文件哈希已验证。
- **发行门禁**。正式 CI 的干净构建、双包 Chromium 以及安装升级回滚卸载留到 `LS-068` 以最终构建运行，标准账户安装现场同属发布验收。

---

## Previous Handoff Snapshot · LS-059

- **Task-ID / Task-Name**：`LS-059`（`DONE`）/ 后续功能、UI 与高性能版本规划。
- **基线**：`main @ a992a42`，延续上一轮尚未提交的五份接力文档。按本轮 `/plan` 更新接力文档，实施任务分批登记为 `READY`。
- **规划**：拟定 `0.5.1` 修发版、实时播放、回滚和首轮 UI，`0.6.0` 完成密码、设备配对、运维与性能基线，`0.7.0` 做多路优化和人脸本地／API 模型管理。上一轮 12 项发现纳入实施任务。详细依赖与通过条件见 `task_plan.md` 的 `LS-060`～`LS-067`。
- **实测**：正式 ZIP 在独立数据目录启动。未设 `LS_UPDATE_MANIFEST_URL` 时，`GET /api/update/check?refresh=1` 返回 `enabled=false`、`current=0.5.0`、`latest=null`、`url=null`，随后服务正常退出。`gh release view` 确认 `v0.5.0` 为最新正式发布。检查更新的网页默认路径规划为直达 Release。
- **参考核验**：`Get-Item`、`Get-AuthenticodeSignature` 和 `Get-FileHash` 确认 EXE 元数据为 FaceWinUnlock-Tauri 0.4.0、未签名。`gh api repos/zs1083339604/FaceWinUnlock-Tauri/releases/tags/0.4.0` 的资产 SHA-256 与本机一致。参考限于文件属性和官方公开的操作流程。
- **Next First Command**：锁定 `LS-060` 后运行 `rg -n 'go2rtc|route coverage' scripts/prepare_rust_assets.py scripts/build_rust.py .github/workflows`，从正式包资产与发布门禁切入。

---

## Previous Handoff Snapshot · LS-058

- **Task-ID / Task-Name**：`LS-058`（`DONE`）/ v0.5.0 接手、全仓复审与正式产物独立验收。
- **基线**：启动时 `main @ a992a42`，工作区干净，Git 提交人 `windyxyz`。重新查询确认远端 `main` 与远端 `v0.5.0` 指向同一提交。[公开 Release](https://github.com/windyxyz/LabSentinel/releases/tag/v0.5.0) 于香港时间 2026-11-22 18:38 发布。
- **审查判断**：源码和发版门禁基本通过。正式产物缺 go2rtc，现有 WebRTC 信令与 1.9.15 不兼容，首次覆盖升级后回滚可失败。本批完成复审与交接记录，待修复事项及验收条件落在 `findings.md` 的 LS-058 小节。
- **本机门禁**：在 `engine-rs` 按 Windows GNU 和纯 ASCII target 配方依次运行 `cargo +stable-x86_64-pc-windows-gnu fmt --all -- --check`、`clippy --workspace --all-targets --all-features -- -D warnings`、`test --workspace --all-features`，三步均退出 0，Rust 184 项通过。Web `npx.cmd vitest run` 15 文件、93 项通过，`npm.cmd run build` 退出 0。`python -X utf8 scripts/contract_audit.py` 通过，117 条前端 API 匹配 155 条后端清单项，安装脚本 BOM 正常。`python -m unittest discover -s tools/tests -v` 11 项通过，`python -m py_compile` 对 `tools/*.py` 退出 0。
- **路由与 CI**：`python -X utf8 scripts/rust_route_coverage.py --exe Resources/dist/rust/labsentinel-server/labsentinel-server.exe --fail-under 95` 退出 0。该统计与 `gh run download 45010145971 --name rust-route-coverage` 一致，106/106，另有 46 条 OpenAPI 清单外路由。`gh run list --commit a992a425a40e0eb49bc72b3543e3ad80b3e77df0` 查到 `tests`、`rust`、`release` 均成功。发布工作流记录包含依赖审计、Windows 静默安装、探活与卸载。本次 Windows 本机执行的是解包及命令行回滚复测。
- **正式产物**：用 `gh release download v0.5.0` 独立下载两件产物和校验清单，`Get-FileHash -Algorithm SHA256` 与发布校验清单一致。ZIP 89,253,263 字节，`1c6abe7755a68a63265a4f18ca7a44461af8da9571cf8a4a6dff4c6080f52f53`。安装器 69,108,068 字节，`3e7472c44798a216641478a5c238a70aa149bd4fbd297d6de15beef72f4ff83b`，Authenticode 为 `NotSigned`。
- **本机同名产物**：ZIP 100,430,452 字节，`5c2d9bdb9f27057cfe8470c7f4fa06d8dffc6169732a330239b4c735dfdafe01`。安装器 75,127,189 字节，`ce06f58a1a61559082eb1d6df0fbb01c4c554c48d0d1714053a6cfa79c5ca526`。时间戳早于 `a992a42`。两份 ZIP 均通过 `ZipFile.testzip()` 和 `python -X utf8 scripts/smoke_rust.py --zip <文件> --timeout 90` 的 5 项启动、API、界面与退出检查。
- **隔离复现**：本机随包 go2rtc 1.9.15 对现有前端 offer 返回 JSON 类型不匹配。正式 ZIP 的服务在 CPU/ORT 上完成同一随包 PPE 模型双路评测，HTTP 200，两个模型会话均返回 `640×640` 和检出。单个已核验样本经正式包 API 导出 HTTP 200，`train=0, val=1`。Python 小样本切分 1～4 张均没有验证样本。正式 exe 在模拟升级目录中归档 0.4.0 返回 0，写入 0.5.0 后回滚返回 2。
- **VLM 本机模拟**：隔离运行 `scripts/acceptance_vlm.py` 的验收逻辑，服务器使用正式 ZIP 二进制，配置 CPU、回环 mock 和独立 DB，通过启动包装去掉打开浏览器的 `--launch` 参数。原素材版第 11b、12 步失败，检查发现 `empty-frame.jpg` 含人员。临时用合成无人图替换后整套通过，空场景 mock 调用保持 `2→2`，加入有人图后 `2→3`。
- **复审过程记录**：首次回滚探针传入 `--console` 为首参数，应用进入服务路径并创建默认空数据配置。两个测试进程已停止。确认相机、事件、设备、样本、设置及投递为零后，将测试创建的整个目录移至临时目录，默认路径复核为不存在。后续回滚探针使用独立测试路径。
- **现场待验证**：真实 RTSP／DirectML 多路与长跑、外部 VLM 和渠道、真实 RT-DETR 导出与全面训练，以及本机交互安装／升级／卸载、标准用户权限和本机设备矩阵。
- **Next First Command**：`rg -n 'LS-058-0[1-6]' findings.md`，从发版资产、真实 WebRTC、回滚和分享连接生命周期缺陷确定修复批次。

---

## Previous Handoff Snapshot · LS-050-057

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
