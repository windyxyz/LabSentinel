# 任务 32 · 模型就绪自检脚本（先读本目录 README.md）

> 依据：docs/45-model-onboarding.md §3 上线门禁。训练完成后接入前需要一键核对，避免手工漏项。

## 目标

新建 `tools/check_model_ready.py`（python，只读检查，不启停服务、不加载大模型跑推理）：

1. **文件存在性**：`m0-spike/yolo11n_ppe.onnx`、`m0-spike/yolo11n_ppe.classes.txt`、`m0-spike/yolo11n_ppe.pt`（若有，仅报告）
2. **classes.txt 校验**：逐行类名、数量与 `datasets/merged/data.yaml` 的 names 完全一致（顺序敏感）；不一致时给出 diff 提示
3. **ONNX 结构校验**：onnxruntime 创建会话（CPU 即可，只建 session 并读 input/output 形状，不推理）；输出通道数应等于 4+类别数
4. **配置校验**：读 `.env`（不打印敏感值），检查 `LS_PPE_MODEL` 指向存在文件、`LS_PPE_CLASS_NAMES`/`LS_PPE_VIOLATION_CLASS` 能在词表中精确匹配；给出当前生效值摘要（路径/类名/阈值）
5. **模型元数据**：打印 ONNX 文件大小、修改时间；若存在 `runs/detect/ppe_v2*/results.csv`，打印最佳 epoch 与 mAP50/mAP50-95（只读）
6. **输出**：PASS/FAIL/WARN 表 + 退出码 0/1；FAIL 项附修复建议（引用 45 手册章节）

## 约束

- 不推理（不触发 DML 并发问题）；session 创建用 CPUExecutionProvider
- 不修改任何文件；不读取 .env 的敏感键值（只判断非空与路径存在）
- 参数：`--root`（默认项目根）、`--model`（默认 m0-spike/yolo11n_ppe.onnx）

## 验收标准

对当前仓库实跑输出 PASS 表（当前 v1 模型 + 10 类或 v2 就绪后 11 类都应能正确判定）；构造一个错误场景（如 classes.txt 少一行）验证 FAIL 与建议输出；执行报告附两次输出摘要。

## 执行报告

### 交付物
- `tools/check_model_ready.py`（只读自检，不启停服务/不推理/不改文件/不打印 .env 敏感值）：
  ① 文件存在性（ONNX/classes.txt/.pt——pt 仅报告）② classes.txt 与 datasets/merged/data.yaml
  逐行顺序敏感比对（不一致给 diff 提示与 v1/v2 语境说明）③ ONNX 结构（CPU 建 session 只读形状，
  校验输出通道 = 4+类别数）④ .env 配置（LS_PPE_MODEL 路径存在性、CLASS_NAMES/VIOLATION_CLASS
  词表精确匹配忽略大小写；敏感值零读取）⑤ 元数据（大小/修改时间 + glob ppe_v2*/results.csv 最佳
  epoch 与 mAP50/mAP50-95）⑥ PASS/FAIL/WARN 表 + 退出码 0/1，FAIL 附 45 手册章节修复建议
- 参数：--root / --model；训练进程并行期间可安全运行（纯只读）

### 实测输出摘要（验收要求两次）
1. **当前仓库（v1 模型）**：`结果：PASS（FAIL 0 / WARN 2 / 总 11 项）`——
   WARN① 词表 10 类 vs merged yaml 11 类（差 HairNet，v1 期间属预期，已注明）；
   WARN② LS_PPE_VIOLATION_CLASS 未设置（服务端默认值）；训练记录读取正常（epoch 35，最佳 0.723@28）
2. **错误场景（词表少一行）**：`结果：FAIL`——
   `[FAIL] ONNX 输出通道与词表不符: out=(1,14,8400)；期望 13（4+9）↳ 修复: 检查 classes.txt 是否对应该 ONNX（45 手册 §1）`

### 验收结果
- 两场景均按预期（PASS/WARN 判定 + FAIL 附修复建议 + 退出码）；--help 可用；py_compile 通过
- 不触发 DML 并发（session 用 CPUExecutionProvider）、不改任何文件、未读敏感键值

### 遗留问题
- v2 完成链路执行后建议复跑本脚本（应为 11 类全 PASS、无词表 WARN）
- data.yaml 对比基准固定为 datasets/merged/data.yaml（--data 参数未暴露，如需对比其他数据集再加）
