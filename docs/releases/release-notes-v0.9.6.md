## LabSentinel v0.9.6 预览版

本版提供 Windows 便携 ZIP 和安装器，随包附带人员与实验室防护用品检测模型。双包未做 Windows 代码签名，打开安装器时可能出现 SmartScreen 提示。下载后可用 `SHA256SUMS-rust.txt` 核对文件。

- 支持实时流、图片文件夹和视频文件的检测留痕。事件可查看截图及已启用的触发录像。
- 白大褂复检按人员归属核对裁剪结果。远景、遮挡和低光下，证据不足的结果会保留为未知。
- 支持按人控制视觉 API 的复核频率，并提供事件问答、报告和本地截图语义检索。
- 提供可选 Vulkan 推理、DirectML 和 CPU 回退，多路分析、大屏及存储清理。
- 支持原生 Windows 服务、本地 MCP 逐工具授权、审计导出及声光联动。

更多版本变更见 [CHANGELOG](https://github.com/windyxyz/LabSentinel/blob/v0.9.6/CHANGELOG.md)。
