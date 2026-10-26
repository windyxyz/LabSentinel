# 任务 11 · 标注工具链（数据飞轮首轮工具，先读本目录 README.md）

> 背景见 [../90-data-flywheel.md](../90-data-flywheel.md)。本任务实现飞轮的 ① ② 两步工具。

## 目标一：`tools/frame_sampler.py`

从视频批量抽帧：
- 参数：`--video <path> --out <dir> --interval 2.0 --max 300`
- 每 `interval` 秒取一帧；用 `cv2.absdiff` 帧差做场景去重（相邻候选帧差异 <阈值 则丢弃，阈值可配 `--diff-thresh 8.0`）
- 输出文件名含来源与秒数：`{video_stem}_{sec:06.1f}.jpg`
- 生成 `manifest.csv`：source, timestamp, frame_file, resolution
- 支持目录批量（`--video-dir`）

## 目标二：`tools/preannotate.py`

用当前部署模型给抽帧做预标注：
- 参数：`--images <dir> --out <labels_dir> --model <onnx> --classes-file <txt>`（classes-file 即 11 类词表，工具不得硬编码类名）
- 按引擎同款 letterbox 预处理（直接复用 `server/app/engine.py` 的 Engine/PpeEngine，**必须走 Engine._run（串行锁）**，不得直接 sess.run——铁律）
- 输出 YOLO 检测框 txt（同名 `.txt`，空检测写空文件）；同时输出 `preview/` 带框 jpg 供快速目检
- 统计打印：每类框数、平均置信度

## 目标三：`tools/README.md` 小节

把两个工具的用法 + "Roboflow 上传预标注"的操作步骤（网页：Create Project → Upload images with annotations → 选 YOLO txt 配对）写清楚，供用户按步骤操作。

## 验收标准

- 用 `m0-spike/data/lab-walk-24076.mp4` 跑通：抽 20 帧、预标注、preview 可看
- 单元测试（不跑模型的部分：manifest 生成、文件名规则、空目录处理）
- 代码中不出现 `sess.run`（grep 验收，必须 `Engine._run`）
- 不改动 server 代码

## 执行报告

### 交付物
1. **`tools/frame_sampler.py`**：`--video`/`--video-dir` 批量抽帧，`--interval` 秒级采样 +
   `cv2.absdiff` 帧差去重（`--diff-thresh` 可配），`--max` 每视频上限；文件名 `{stem}_{秒:06.1f}.jpg`；
   生成 `manifest.csv`（source,timestamp,frame_file,resolution）；`grab()+retrieve()` 跳帧加速。
2. **`tools/preannotate.py`**：一次前向解全部类别（letterbox + **`engine._run`（全局串行锁）** + NMS），
   输出同名 YOLO txt（空检测写空文件）+ `preview/` 带框目检图；打印每类框数与平均置信度；
   `--classes-file` 支持一行一个类名或直接给 `data.yaml`（解析 names）；中文路径用 `imdecode` 规避 cv2.imread 限制。
3. **`tools/README.md` 第七节**：两工具用法 + 参数建议 + Roboflow 上传预标注/修正/合训的五步流程 + 必须人工复核提示。

### 验收结果
- `pytest tests/ -q`：**41 passed**（新增 4 个非模型单测：manifest/文件名规则/坏路径/空目录/类名文件解析）
- 实测（演示视频）：`frame_sampler` 抽 20 帧 + manifest 正常；`preannotate` 20 张图跑通，
  日志可见 `(accel: run 串行锁启用)`；输出 20 个 txt + preview；类统计 Person 16 / Lab Coat 6 / Glasses 7 等
- `grep -c "sess.run" tools/preannotate.py` = 0（注释措辞已避开该 token；代码只走 `engine._run`）
- 未改动 server/ 代码；两脚本 `--help` 正常

### 遗留与如实记录
- **演示视频（mixkit 域外素材）上预标注质量有限**：置信度普遍 0.25-0.58，出现把机器区域框成 Person 的误检、
  中间人物漏检——与 roadmap 已记录的"域外视频置信度保守"一致。结论：飞轮应使用**真实实验室摄像头抽帧**，
  且预标注必须人工过一遍（尤其 Lab Coat / No Lab Coat 混淆样本）。
- 合并脚本 `datasets/README` 提到的多数据集合并脚本本次未见（可能未入库）；上传 Roboflow 后合训前需确认其存在。
