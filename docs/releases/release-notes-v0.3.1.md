# Release Notes · LabSentinel v0.3.1

> **未签名预览版**——安装时 SmartScreen 可能提示，请核对 `SHA256SUMS-rust.txt`。
> 发行形态：Rust 单二进制 zip + Inno Setup 7 安装包。相对于 0.3.0 仅两项修复/增强。

## 本版变化

### 实验服（PPE）模型随包发布
- `yolo11n_ppe.onnx` 与类别文件已随 zip / setup.exe 分发，**安装即启用
  「人员 + 实验服检测」**，无需在网页端确认许可后下载。
- 此前 0.3.0 安装包不含该模型、下载源 404、也没有 `assets.lock.json`，用户只能
  看到「仅人员检测」和「下载失败」。
- 设置 → 模型管理 现在显示「随安装包提供，开箱即用」，并给出实际文件
  SHA256 与大小。
- 许可：权重由 LAB PPE 数据集（Roboflow Universe，**CC BY 4.0**）训练派生，
  署名与哈希见包内 `NOTICE.txt`。

### 网页版本号修复
- 仪表盘「版本」卡片此前恒显示 `0.2.0`（后端兜底写死），现显示实际版本
  （`0.3.1`），「检查更新」的当前版本口径同步修正。

### 卸载清理优化
- 卸载时清理 `versions\` 升级归档整树、旧布局 `server\`/`data\`、旧启动器锁文件，
  并尽力清理 `Prefetch`（系统缓存，仅管理员卸载时可删）。
- 用户数据默认保留；交互卸载新增「是否同时删除用户数据（数据库/截图/备份/密钥）」
  询问，静默卸载用 `/REMOVEDATA=1` 显式删除。
- Windows 自身缓存项（MuiCache / BAM / CloudStore 等）由系统维护，卸载器不处理。

## 升级说明

- 直接覆盖安装即可，数据目录 `%LOCALAPPDATA%\LabSentinel\data` 不受影响。
- 旧版若手动配置过 `LS_PPE_MODEL`，仍以显式配置优先；否则自动使用包内模型。

## 验证基线（2026-11-21）

| 项 | 结果 |
|---|---|
| Rust workspace | 136 passed |
| Web（vitest） | 52 passed |
| 安装包 | 安装 → 根路径 200 → ready → 卸载零残留 |
| 随包模型 | `models/yolo11n_ppe.onnx` SHA256 `f00231e6…52dd6f` 与锁文件一致 |
