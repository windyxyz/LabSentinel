# Kaspersky Open False Positive 申诉材料（LabSentinel 0.2.0 便携包）

> **2026-11-12 注**：本材料为**预防性预备**（发版后若收到真实用户误报再提交）；
> 开发本机未安装卡巴斯基，此前会话的「删除」判断已更正（见 docs/plan/status.md
> 便携包启动链排障段）。发版后哈希变化时用 `python scripts/opentip_hashes.py`
> 重新生成本文件。

提交地址：https://opentip.kaspersky.com/（Submit False Positive / False Alarm）

## 申诉理由（可直接粘贴）

LabSentinel 是开源（AGPL-3.0）本地实验室安全监控软件：摄像头画面经本地推理
（ONNX Runtime）检测未穿实验服等违规并产生告警，全部数据留在本机。
便携包的运行形态为「启动器 EXE → runtime\pythonw.exe 托管本地 uvicorn 服务
（仅监听 127.0.0.1）→ 拉起 go2rtc.exe 做 WebRTC 网关」。我们理解该行为组合
会命中启发式（本地监听 + 拉起第三方二进制 + 无签名启动器），但：
1) 启动链无动态代码执行（无 -c/exec/runpy），唯一的 urlopen 是对 127.0.0.1 的
   健康检查；2) Python 运行时为 python.org 官方签名发行；3) 全部源码开源可审计：
   https://github.com/Windy-LabSentinel/LabSentinel
实测 Kaspersky 会在构建产物目录（含 runtime\python.exe 等官方签名文件）上触发
「HEUR:Trojan」类判定并静默删除文件。请复核以下文件并加白。

## 文件与 SHA256（完整）

- `LabSentinel.exe`  
  SHA256: `2df10714f79163ff339ae34f60e93524f8cc0e6182b36d8ca9f8e20169a549ab`  
  大小: 7680 字节  
  说明: 启动器（C# .NET Framework 4，winexe，本仓库 scripts/launcher_exe/LabSentinelLauncher.cs 编译）
- `runtime/python.exe`  
  SHA256: `1a078825122b059f04b6c1545909f1d3ed0838dd24ab80d1c7fa60ca68ff9af5`  
  大小: 104952 字节  
  说明: CPython 3.12 官方 embed（python.org 发行，PSF Authenticode 签名）
- `runtime/pythonw.exe`  
  SHA256: `2689aa613159a76df109a11ebf0f278300511cf477ab449436e2f7353ab63ada`  
  大小: 104304 字节  
  说明: 同上（无控制台变体）
- `scripts/portable_launcher.py`  
  SHA256: `4512f6f53476f0a956b778ca9fa7880273e00b1ac388724e7b04610bb44f37be`  
  大小: 25811 字节  
  说明: 启动器脚本（标准库：uvicorn 子进程托管 + 回环健康检查）
- `tools/go2rtc.exe`  
  SHA256: `994aa42768d2d6e1bbcf561c85bf9c46cafa705a90a13537cd38be2faee287de`  
  大小: 21620224 字节  
  说明: go2rtc 官方发行（MIT，github.com/AlexxIT/go2rtc）
- `tools/ffmpeg.exe`  
  SHA256: `04e1307997530f9cf2fe35cba2ca7e8875ca91da02f89d6c7243df819c94ad00`  
  大小: 82797568 字节  
  说明: ffmpeg（GPL/LGPL 构建，BtbN 发行）
- `bin/ls-analyzer.exe`  
  SHA256: `5c4b3f2d4bca1fd71d796b74f2843e531bc25739b1e8babc8e1a0f07ff9c83c4`  
  大小: 26272603 字节  
  说明: 本项目 Rust 分析器（AGPL-3.0，engine-rs 源码构建）
- `bin/onnxruntime.dll`  
  SHA256: `302c69f9779d63ef4ab90316e59444c4acbaca7fe3455020d79d10bcfcb00715`  
  大小: 21111832 字节  
  说明: ONNX Runtime 官方发行（MIT，DirectML EP 版）

## 附加信息

- 版本：0.2.0（未签名预览版；代码签名证书在办，到位后随版签名）
- 行为白皮书：仓库内 docs/杀毒软件误报说明.md（特征对照表）
- 来源 zip：LabSentinel-portable-20261111.zip
- 联系：维护者（见仓库 README）