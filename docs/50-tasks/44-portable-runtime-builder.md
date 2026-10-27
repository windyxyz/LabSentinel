# 任务 44 · 便携运行时构建器（嵌入式 Python + 依赖 + 资产）

> 背景：任务 43 让服务端能单端口托管 UI；本任务把「Python 环境」也打进包：用户侧不需要装 Python/pip，双击即可运行。方案见 [docs/58-packaging-strategy.md](../../58-packaging-strategy.md)。

## 目标

1. **新增 `scripts/build_portable.py`**（标准库 + subprocess，风格同 deploy.py）：
   - 下载 CPython **3.12.x embed-amd64**，源按顺序回退：
     1) `https://registry.npmmirror.com/-/binary/python/3.12.8/python-3.12.8-embed-amd64.zip`
     2) `https://mirrors.huaweicloud.com/python/3.12.8/python-3.12.8-embed-amd64.zip`
     3) `https://www.python.org/ftp/python/3.12.8/python-3.12.8-embed-amd64.zip`
     （已验证三者均可达；下载后校验 zip 完整性）
   - 解压到 `dist/portable/LabSentinel/runtime/`；写 `python312._pth`：
     ```
     python312.zip
     .
     Lib\site-packages
     ```
   - **预装依赖**：用项目 `.venv` 的 pip 执行
     `pip install --target runtime/Lib/site-packages -r server/requirements.txt`
     （清华 → 阿里云 → 官方回退；`--no-warn-script-location`；不要用 `--user`）。
   - **复制资产**：
     - `server/app/`（只代码；排除 `server/data`、`__pycache__`、tests 可选）
     - `web/dist/` → `app/web/`（若不存在先提示执行 `npm run build`，构建脚本不代跑 npm）
     - `m0-spike/yolo11n.onnx`、`yolo11n_ppe.onnx`、`yolo11n_ppe.classes.txt` → `models/`
     - `tools/ffmpeg/ffmpeg.exe` → `tools/`（ffprobe 可选，61MB，默认不带）
     - `tools/go2rtc.exe` → `tools/`
     - `LICENSE`、`NOTICE`、`server/.env.example`
   - 生成 `app/server/.env.template`：模型路径用相对 `../../models/...`（启动器会复制到数据目录并改绝对路径，见任务 45；本任务只要模板存在）。
   - **冒烟（PATH 剥离）**：
     ```bat
     set PATH=C:\Windows\System32
     runtime\python.exe -c "import fastapi,uvicorn,cv2,numpy,supervision,onnxruntime; print(onnxruntime.get_available_providers())"
     ```
     要求输出包含 `DmlExecutionProvider` 或可回退 CPU；再在 `app/server` 下
     `runtime\python.exe -c "import app.main"`（或 `-m uvicorn --help`）验证导入。
   - 打印产物大小（runtime/site-packages/models/tools/合计）。
2. **打包 zip**（可选但建议）：`dist/LabSentinel-portable-<日期>.zip`，zip 内根目录为 `LabSentinel/`；
   `dist/` 已在 .gitignore，不提交大包。
3. **`.gitignore`**：确认 `dist/` 已忽略；新增产物不污染仓库。
4. **报告**：命令、下载源与耗时、各目录体积、PATH 剥离冒烟输出、已知缺失（ffprobe/tests/图标）。

## 约束

- 只写 `scripts/build_portable.py` + 报告/文档；不改业务代码（单端口是任务 43 的前置，若 43 未合并则本任务只验证导入，不启动 UI）。
- 下载全部走镜像回退；一次一个重任务；不训练；不占 8000/5173。
- 不把 `server/data`、`.env`（真实配置）、`__pycache__`、`web/node_modules` 打进包。
- 模型/二进制仅复制，不改 `m0-spike/`。

## 验收标准

- `dist/portable/LabSentinel/runtime/python.exe` 存在，`import` 冒烟在 `PATH=System32` 下通过；
- `app/web/index.html`、`models/*.onnx`、`tools/ffmpeg.exe`、`tools/go2rtc.exe` 齐全；
- 体积报告合理（runtime+site-packages 约 600MB，总计约 700MB 量级）；
- 脚本可重复执行（幂等，已存在则跳过下载/安装）；
- 执行报告含命令、源、耗时、体积、冒烟输出。

## 执行报告

（待填：下载源/耗时 / 目录体积 / PATH 剥离冒烟 / zip 产物 / 已知缺失）
