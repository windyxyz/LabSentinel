# 任务 40 · 演示资产收尾（L1 命名冲突 / L2 类别绘制 / L4 抽样图）

> 背景：任务 35 的 `docs/53-v2-demo-report.md` §6 留了 4 条建议。本任务处理其中 L1/L2/L4；L3（长视频补源）暂缓，保留在报告里。

## 目标

1. **L1 修复 `效果演示/make_demo.py` 输出命名**：
   - 现在 `basename.split('-')[0]` 把 `lab-walk-24076` 和 `lab-equip-21453` 都截成 `lab`，后写覆盖先写。
   - 改为显式映射 + 安全回退：
     ```python
     NAME_MAP = {"lab-walk-24076": "lab-walk", "lab-equip-21453": "lab-equip"}
     stem = NAME_MAP.get(base, re.sub(r"-\d+$", "", base))
     ```
   - 输出名：`{stem}-检测效果{suffix}.mp4` → `lab-walk-检测效果_v2.mp4`、`lab-equip-检测效果_v2.mp4`。
   - 同步更新所有引用旧名（`lab-检测效果_v2.mp4`、`实验有风险，防护需穿戴-检测效果_v2.mp4`）的文档：至少 `docs/53-v2-demo-report.md`；全仓 grep 一遍（README/docs/快速开始）。
   - **旧文件不要删除**（用户可能已对外展示）；在报告里列出新旧对照，标注旧文件为"历史命名"。
2. **L2 绘制类别扩展**：
   - 增加 `--draw` 参数（逗号分隔，默认 `lab coat,no lab coat`）；person 始终绘制。
   - 为 11 类建立颜色表（至少 Gloves/HairNet/Glasses/Mask/Shoes 有区分色）；未知类名直接报错退出，避免静默画错。
   - 默认行为与现在一致（不增加视觉噪音），需要时可用 `--draw "lab coat,no lab coat,hairnet"`。
3. **L4 抽样图**：
   - 抽样 jpg 默认关闭；加 `--samples` 才写，输出到 `logs/demo_samples/`（已被 .gitignore 忽略），不再写 `效果演示/`。
   - 确认 `scripts/make_demo_zip.py` 不会打入这些 jpg（本来就只扫 `效果演示/`，但请实测一次打包清单或用 `zipfile.namelist()` 检查）。
4. **重生成**（CPU，避免抢 GPU）：
   ```bat
   D:\LabSentinel\m0-venv\Scripts\python.exe 效果演示\make_demo.py --device cpu --suffix _v2
   ```
   校验：两段新名字 mp4 存在、h264、时长与源一致、非 0 字节；`--help` 含 `--draw/--samples`。
5. 更新 `docs/53-v2-demo-report.md`：L1/L2/L4 标记已解决，L3 保留；附新旧文件名对照与复现命令。

## 约束

- 只改 `效果演示/make_demo.py` 与文档；不删旧 mp4；不碰 `m0-spike/` 模型/视频；不训练；不占 8000/5173。
- 单任务串行，CPU 推理（`--device cpu`）。
- 若重生成耗时明显（>10 分钟/段），可只跑短视频 + 长视频沿用旧文件，但必须在报告说明。

## 验收标准

- `--help` 含 `--draw`、`--samples`；未知类别报错；
- 两段 v2 输出使用新命名且互不覆盖（文件名含 lab-walk / lab-equip）；
- `logs/demo_samples/` 生成（仅当 `--samples`），`效果演示/` 不再新增 jpg；
- docs/53 更新（L1/L2/L4 已解决）；旧文件保留并标注；
- 执行报告列新旧文件名对照与命令耗时。

## 执行报告

（待填：改动 / 命令与耗时 / 新旧文件对照 / 打包检查 / 遗留 L3）
