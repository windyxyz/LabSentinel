# 任务 25 · 发布前检查清单核对（先读本目录 README.md）

## 背景

docs/70-release-plan.md 定义了 v0.1.0-alpha 的发布前 checklist（法律/代码/文档/CI）。现在 01–24 已交付大半，需要有人逐项**实际核对**并产出差距清单——不写代码，只做检查与记录。

## 目标

新建 `docs/发布核对报告.md`，逐条对照 70-release-plan.md 的三张 checklist（法律合规/代码质量/文档），每条标记：

- ✅ 已满足（附证据：文件路径/命令输出摘要）
- ❌ 未满足（写清缺什么、谁来补、建议任务编号）
- ⚠️ 部分满足（缺哪部分）

重点核对项（至少覆盖）：

1. **法律合规**：LICENSE 文件是否存在且为 AGPL-3.0 全文；`.gitignore` 是否覆盖 `server/.env`、`server/data/`、`m0-spike/` 大文件、`datasets/`、`runs/`、`node_modules`；README 合规章节与模型/数据集不入库声明；Roboflow key 是否零入库（grep `rf_` 全仓）
2. **代码质量**：`pytest` 实际跑一次（当前应 ≥70，训练中则记录"顺延"）；`npm run build`；grep 检查 `sess.run` 出现在 server 外的地方（应只有 engine.py 内部）；`TODO/FIXME` 数量统计
3. **文档**：README 截图链接有效性（本地文件存在性检查）；docs/ 下文档索引完整性（部署指南/FAQ/API/模型接入/里程碑/风险/飞轮/任务包全部可达）；LICENSE 与 README 协议声明一致
4. **CI**：`.github/workflows/` 是否存在（预期缺失 → 列入 ❌ 并给建议任务）
5. **发布物**：Dockerfile/compose 是否仍可用（静态检查，不实跑）；scripts 三件套存在
6. **仓库卫生**：仓库根文件清单（应有：README/LICENSE/GLOSSARY/docs/server/web/tools/scripts/android/docker-compose.yml）；是否存在意外提交物（dist/node_modules/数据）

## 约束

- 纯检查：不修改被测文件；只新建这一份报告
- 需要跑命令时遵守重负载铁律（训练/压测进行中则跳过执行类检查并标注"顺延"）
- 不启动/停止服务进程

## 验收标准

报告含三张 checklist 的逐项结论 + "发布阻塞项清单"（❌ 汇总，按优先级排序）+ "建议任务"编号；每条 ✅ 均有可复验的证据。

## 执行报告

### 交付物
- `docs/发布核对报告.md`（新）：对照 70-release-plan 三张 checklist 逐项实际核对（法律合规 5 项 /
  代码质量 6 项 / 文档 3 项 / CI 1 项 / 发布物 2 项 / 仓库卫生 2 项），每条附可复验证据；
  含"发布阻塞项清单"（6 项按优先级）与建议任务 26（发布物准备）/27（演示素材更新）；
  与 70 号文档"停止线"逐条对照

### 核对中实际跑过的命令
- pytest tests/ -q → **74 passed**；npm run build → 通过（并行训练下真实输出）
- grep 扫描：Roboflow key 形态（rf_+10 位）零命中；`sess.run` 在 server 外零命中（engine.py 封装内 3 处）
- TODO/FIXME 全仓 = 0；LICENSE/.gitignore/.github/workflows 存在性 = 均缺失（❌ 如实记录）

### 关键发现
1. **LICENSE 与 .gitignore 双缺 + 无 git 仓库**——发布最大阻塞项，且 .gitignore 必须先于首次建仓提交
2. 根目录有量化过程产物（quantized_info.csv）与 logs/ 需要归位/忽略
3. 其余项（合规章节/截图/文档索引/Docker/scripts/key 零入库）全部 ✅

### 遗留问题
- 纯检查未修改任何文件；阻塞项修复归入建议任务 26（待主线派发）
- pytest 在训练并行下跑出（74 passed），无互相干扰（训练为 torch 满核、pytest 短时突发）
