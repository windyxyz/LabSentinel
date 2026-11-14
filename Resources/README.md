# Resources

非项目源码文件统一归集目录。根目录只保留项目文件：`engine-rs/`、`web/`、`android/`、
`tools/`、`scripts/`、`docs/`、`m0-spike/` 与入口文件（README/一键启动.bat 等）。

| 子目录 | 内容 | 入库状态 |
|---|---|---|
| `效果演示/` | 检测效果演示视频（`make_demo.py` 生成）与生成脚本 | 脚本入库；mp4/jpg 不入库（Mixkit 派生素材仅个人使用） |
| `examples/` | 插件示例（event-logger 等），配合 docs/dev/plugin-guide.md | 入库 |
| `datasets/` | 训练/评估数据集（Roboflow 下载、抽帧、标注产物） | 不入库（1.4GB+） |
| `runs/` | 训练产物（Ultralytics runs，best.pt 等） | 不入库 |
| `logs/` | 构建/验收脚本的输出与缓存 | 不入库 |
| `dist/` | 发行包构建产物（`build_rust.py --zip` / `prepare_rust_assets.py`） | 不入库 |
| `acceptance/` | 验收素材（VLM 验收用图片帧等） | 入库（小体积） |

服务端运行数据（事件库/截图/视频/告警）默认在 `data/` 或
`%LOCALAPPDATA%\LabSentinel\data`，与本目录无关。
