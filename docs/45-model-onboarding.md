# 45 · 模型接入手册（新模型 → 上线，照做即可）

> v0.1（2026-10-26）· 场景：v2/v3 模型训练完成后，**零改代码**接入现有系统。
> 前置：训练工具链见 `tools/train_ppe.py`；评估见 `tools/eval_report.py`；门禁见 60-milestones M1.5。

## 0. 命名与放置约定（唯一约定，违反会导致服务端找不到模型）

| 角色 | 文件 | 位置 | 说明 |
|---|---|---|---|
| 人员检测 FP32 | `yolo11n.onnx` | `m0-spike/` | COCO 80 类，person=0；DirectML/CPU 用 |
| 人员检测 XINT8（可选） | `yolo11n_xint8_hd.onnx` | `m0-spike/` | NPU 后端用 |
| 实验服模型 | `yolo11n_ppe.onnx` | `m0-spike/` | 11 类合并词表（见下） |
| 实验服词表 | `yolo11n_ppe.classes.txt` | `m0-spike/` | **一行一类，顺序=模型输出顺序**（PpeEngine 读取；缺失则用 .env 兜底） |

**classes.txt 是硬契约**：v2 合并模型词表（训练时确定，勿手改顺序）：
```
Glasses
Gloves
Lab Coat
Mask
No Glasses
No Gloves
No Lab Coat
No Mask
Person
Shoes
HairNet
```

## 1. 训练完成后的导出（标准流程）

```bash
# 1) 评估（见执行报告要求：逐类 AP50 截图/表格）
python tools/eval_report.py --weights runs/detect/<run>/weights/best.pt --data datasets/merged/data.yaml

# 2) 导出 ONNX（opset13、imgsz640、nms=False——服务端自己后处理）
python tools/export_onnx.py --weights runs/detect/<run>/weights/best.pt --out m0-spike/yolo11n_ppe.onnx

# 3) 词表落盘（从训练 data.yaml 生成，不要手敲）
python -c "import yaml;print('\n'.join(yaml.safe_load(open('datasets/merged/data.yaml'))['names']))" > m0-spike/yolo11n_ppe.classes.txt
```

## 2. 服务端接入（零代码改动）

`server/.env`：
```ini
LS_PPE_MODEL=D:/User/Desktop/TOOLS/监控识别/m0-spike/yolo11n_ppe.onnx
LS_PPE_CLASS_NAMES=lab coat        # 穿戴证据类别（精确匹配优先）
LS_PPE_VIOLATION_CLASS=no lab coat # 违规证据类别（只有它触发告警）
LS_PPE_CONF=0.30                   # 违规判定阈值
LS_PPE_WEARING_CONF=0.20           # 穿戴证据阈值
```

重启服务端 → 启动日志应出现：
```
[engine] backend=directml model=...yolo11n_ppe.onnx init=0.2s (accel: run 串行锁启用)
[engine] ppe model loaded: ...yolo11n_ppe.onnx
```
（当前 PpeEngine 的 names 若仍硬编码，说明 60-milestones M1.5 #2 未做——先完成该参数化任务，再走本手册。）

## 3. 上线前门禁（全部通过才算接入完成）

- [ ] **评估门禁**：test split Lab Coat AP50 ≥0.90、Person ≥0.95；No Lab Coat 误报（穿者被判未穿）≤5%
- [ ] **真实素材对比**：`效果演示/make_demo.py` 重跑用户素材，生成 v1/v2 对比数据（检出帧率/平均置信度/误报条数）
- [ ] **垂直切片回归**：文件摄像头重放 → 事件 detail 正确输出"已穿/未穿/不确定"，`enter_no_ppe` 只在未穿时触发（证据分级！）
- [ ] **串行锁验证**：`pytest tests/test_core.py::test_accel_run_serializes -v` 通过；确认推理走 `Engine._run`
- [ ] **24h 长稳**（真实摄像头）：无 Kernel-Power 41、无 worker 崩溃、内存无持续增长
- [ ] **回滚预案**：旧模型改名备份（`yolo11n_ppe.v1.onnx`），新模型出问题 5 分钟内可切回

## 4. 灰度与回滚

1. 新模型先跑**文件源回放**（零风险）
2. 再挂**一路真实摄像头**观察 24h（告警抽查准确率 ≥80%）
3. 全量切换；保留旧模型与 `classes.txt` 快照
4. 回滚 = 恢复旧 ONNX + classes.txt，重启服务端（无需改代码）

## 5. 常见问题

| 现象 | 原因 | 处理 |
|---|---|---|
| `ppe model loaded` 未出现 | 路径错误/文件不存在 | 检查 .env 绝对路径与反斜杠转义 |
| 检测全 0 | 词表顺序错（classes.txt 与模型不符） | 用 data.yaml 重新生成 classes.txt |
| 'lab coat' 命中 'No Lab Coat' | 匹配逻辑退化为子串 | 确认 detect_class 精确匹配优先（已实现）；类别名大小写核对 |
| 置信度普遍偏低 | 域差距/量化误差 | 走数据飞轮（90-data-flywheel.md）而不是硬降阈值 |
| NPU 后端推理报错 | XINT8 模型/量化不符 | NPU 为可选，回退 directml 即可 |
| 换模型后内存暴涨 | 旧 session 未释放（进程未重启） | 必须重启服务端（uvicorn 不热换模型） |
