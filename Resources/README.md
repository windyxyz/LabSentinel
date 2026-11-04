# Resources

非项目源码文件统一归集目录（2026-11-05 目录重构）。根目录只保留项目文件：`server/`、`web/`、
`android/`、`tools/`、`scripts/`、`docs/`、`m0-spike/` 与入口文件（README/快速开始/一键启动.bat 等）。

| 子目录 | 内容 | 入库状态 |
|---|---|---|
| `效果演示/` | 检测效果演示视频（`make_demo.py` 生成）与生成脚本 | 脚本入库；mp4/jpg 不入库（Mixkit 派生素材仅个人使用） |
| `examples/` | 插件示例（event-logger 等），配合 docs/74-plugin-guide.md | 入库 |
| `datasets/` | 训练/评估数据集（Roboflow 下载、抽帧、标注产物） | 不入库（1.4GB+） |
| `runs/` | 训练产物（Ultralytics runs，best.pt 等） | 不入库 |
| `logs/` | 部署/构建脚本的下载缓存与输出（服务端运行日志在 `server/data/logs/`，不在这里） | 不入库 |
| `dist/` | 便携包构建产物（`build_portable.py --zip` 输出、RELEASE-MANIFEST.json） | 不入库 |

服务端运行数据（事件库/截图/视频/告警）在 `server/data/`，与本目录无关。
