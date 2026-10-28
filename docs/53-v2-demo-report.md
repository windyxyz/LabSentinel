# 53 · v2 效果演示收尾报告（任务 35 / 勾销任务 27）

> 日期：2026-10-28 00:20。执行：flash（任务 35）。前置：v2 模型已接入（10-27 07:41 完成链路）。
> 结论先行：**两段 v2 演示视频已转为可复现产物（h264），v1/v2 对比帧与 test 集指标互相印证——v2 主要收益是误报大幅下降（No Lab Coat P 11%→77%），单帧存在边界摇摆属预期。**

## 1. 产物清单

| 文件 | 编码 | 时长 | 大小 | 来源 |
|---|---|---|---|---|
| `效果演示/lab-检测效果_v2.mp4` | h264 | 16.95s | 4.2MB | **本次 CPU 重新生成**（源：lab-equip-21453.mp4，508 帧）+ x264 转码 |
| `效果演示/实验有风险，防护需穿戴-检测效果_v2.mp4` | h264 | 64.23s | 8.5MB | 10-27 产物（mpeg4）**x264 转码**；源视频（1927 帧@30fps）已不在仓库，无法重推理 |
| `效果演示/lab-检测效果_v1.mp4` | mpeg4 | 16.95s | 12.7MB | 本次 CPU 生成（v1 备份模型），仅作对比素材，不随发布物分发 |
| `docs/assets/v2-compare/*.png` | — | — | 4 张 | v1|v2 左右拼接对比帧（见 §3） |

两段 v2 成品均可播放（h264 / yuv420p），短视频 16.95s≥10s ✓，长视频 64.23s≥30s ✓。

## 2. 复现命令与耗时

```bat
cd /d D:\User\Desktop\TOOLS\监控识别

rem 短视频（CPU 推理 + 每帧标注，约 90s）
D:\LabSentinel\m0-venv\Scripts\python.exe 效果演示\make_demo.py --device cpu --videos m0-spike\data\lab-equip-21453.mp4 --suffix _v2

rem 对比用 v1（同源，约 90s）
D:\LabSentinel\m0-venv\Scripts\python.exe 效果演示\make_demo.py --device cpu --ppe m0-spike\yolo11n_ppe_v1.onnx --videos m0-spike\data\lab-equip-21453.mp4 --suffix _v1

rem h264 转码（秒级；mp4v 直出体积大 3 倍且不符合发布要求）
tools\ffmpeg\ffmpeg.exe -y -i 效果演示\lab-检测效果_v2.mp4 -c:v libx264 -preset medium -crf 23 -pix_fmt yuv420p out.mp4
```

`--device` 说明：`directml`（默认，快）/ `cpu`（慢但结果一致，用于避免与其它推理抢 GPU）。person 引擎走 `Engine(prefer=...)`，PPE 引擎经 `settings.backend_order` 切换；本次全 CPU 实测，`[engine] backend=cpu` 确认生效。

### 生成机制的勘误（重要）

- 旧 `lab-检测效果_v2.mp4`（10-27，16.95s）**实际来源是 lab-equip-21453.mp4**，不是文件名直觉的 lab-walk：`make_demo.py` 的 stem 截断逻辑把两个默认源都截成 `"lab"`，第二个视频覆盖第一个（**遗留建议 L1**）。
- lab-walk-24076.mp4 源只有 7.84s（196 帧@25fps），低于"短视频≥10s"验收线，故正式产物以 lab-equip 源为准。
- 长视频源已缺失，10-27 的 mpeg4 产物直接转码为 h264（画面内容不变，无重新推理）。

## 3. v1/v2 关键帧对比（4 组，`docs/assets/v2-compare/`）

逐 0.5s 扫描两段源视频的双模型检测差异后选取（图文一致，帧号精确定位）：

| 对比帧 | v1 表现 | v2 表现 | 解读 |
|---|---|---|---|
| [walk_t1.0s_v1miss_v2hit.png](../assets/v2-compare/walk_t1.0s_v1miss_v2hit.png) | 无检出（漏检） | lab coat 0.37 | v2 对远处走入的白大褂更敏感 |
| [walk_t4.8s_conflict.png](../assets/v2-compare/walk_t4.8s_conflict.png) | lab coat 0.57 | no lab coat 0.51 | **边界帧摇摆**：两者在 4.3–5.8s 区间来回翻转，属单帧不稳定（见 §5） |
| [walk_t7.2s_conf.png](../assets/v2-compare/walk_t7.2s_conf.png) | lab coat 0.79 | lab coat 0.91 | 检出一致时 v2 置信度更高、框更贴合 |
| [equip_t5.6s_v1fp.png](../assets/v2-compare/equip_t5.6s_v1fp.png) | 误报 no lab coat 0.33（红框画在蓝衣人物后脑勺） | 正确静默 | **v2 核心收益**：蓝大褂/便装不再被误判为违规；逐帧扫描显示 v1 在 lab-equip 全程误报 8 个时间点，v2 零误报 |

## 4. 与 test 集指标互相印证（引用 docs/52-model-eval.md，491 图）

| 类别（精确率 P） | v1 | v2 | 与演示帧的对应关系 |
|---|---|---|---|
| Person | 32% | **51%** | — |
| Lab Coat | 22% | **49%** | walk_t7.2s：0.79→0.91 |
| No Lab Coat | 11% | **77%** | equip 组：v1 全程误报 → v2 零误报 |
| HairNet（新增类） | — | **82%** | — |

单帧表现与集合指标方向一致：v2 的提升主要来自**误报收敛**，而非"每帧都更准"。

## 5. 已知局限

1. **单帧摇摆**：远离镜头/转身/边界光照帧上，lab coat 与 no lab coat 会在 0.2~0.6 置信区间互相翻转（walk_t4.8s 即实例）。服务端事件层已按连续帧聚合 + 阈值判定，偶发单帧翻转不会直接产出违规事件，但意味着演示视频里红框可能出现后消失。
2. **演示绘制仅 3 类**：make_demo 只画 person / lab coat / no lab coat；Gloves、HairNet 等 11 类中的其余类别已检测但不绘制（遗留建议 L2）。
3. **长视频不可重推理**：其源视频不在仓库，若需更新该段内容须先补源（遗留建议 L3）。
4. 蓝色工作服场景天然不触发白大褂违规，v2 静默是**正确行为**，勿误读为"漏检"。

## 6. 遗留建议

- ~~**L1** stem 截断~~：**任务 40 已修复**——NAME_MAP 显式映射 + `re.sub(r"-\d+$")` 回退；输出名 lab-walk/lab-equip-检测效果*.mp4（旧名文件保留为历史命名）。
- ~~**L2** 绘制类别扩展~~：**任务 40 已修复**——`--draw` 参数（默认 lab coat,no lab coat 不变）+ 11 类颜色表，未知类别报错退出。
- **L3** 补充长视频源素材或改用 lab-walk+lab-equip 拼接替代。
- ~~**L4** 抽样图~~：**任务 40 已修复**——默认不写 jpg；`--samples` 才输出到 `logs/demo_samples/`（gitignored，打包清单已实测不含）。

## 执行报告（任务 35）

- **改动文件**：`效果演示/make_demo.py`（+`--device` 参数，person 走 prefer、PPE 走 backend_order，帮助文本注明用途）；新增 `docs/53-v2-demo-report.md`、`docs/assets/v2-compare/` 4 张对比图；重生成/转码 `效果演示/` 两段 v2 mp4。
- **实际命令与耗时**：见 §2；v2 短视频 CPU 生成 ≈90s、v1 ≈90s、转码 ≈10s；逐帧扫描脚本（0.5s 步进×2 源×2 模型，CPU）≈3min。
- **验收对照**：`--help` 含 `--device` ✓；两段 v2 mp4 = h264 / 16.95s≥10s / 64.23s≥30s / 非 0 字节 ✓；本报告含复现命令、4 组对比帧、test 指标引用、局限 ✓；调度看板已勾销任务 27 ✓。
- **过程记录**：初次用默认两源生成时发现 stem 冲突与时长不符（7.84s<10s），溯源后改用 lab-equip 源重新生成；全部推理走 CPU（`--device cpu`），未与其他任务并发，未占 8000/5173，未改 m0-spike。
- **遗留问题**：§6 L1–L4。

## 任务 40 收尾更新（2026-10-28）

- **新命名产物**（互不覆盖，`--device cpu --suffix _v2` 重生成 + x264 转码）：
  - `效果演示/lab-walk-检测效果_v2.mp4`（h264 / 7.84s / 196 帧）
  - `效果演示/lab-equip-检测效果_v2.mp4`（h264 / 16.95s / 508 帧）
- **历史命名对照**（保留未删，内容为新命名产物的同源早期版本）：
  - `效果演示/lab-检测效果_v2.mp4`（旧 stem 截断命名，h264，= lab-equip 源）
  - `效果演示/实验有风险，防护需穿戴-检测效果_v2.mp4`（源视频已缺失，仅存产物）
  - `效果演示/lab-检测效果_v1.mp4`（v1 对比素材）
- **新增能力**：`--draw`（11 类颜色表，未知类别报错）、`--samples`（抽样 jpg →
  `logs/demo_samples/`，gitignored；打包清单实测不含）、cv2.imwrite 中文路径静默失败改
  imencode+tofile。
- 复现命令不变（见 §2），另加 `--draw "lab coat,no lab coat,hairnet"` 示例。
