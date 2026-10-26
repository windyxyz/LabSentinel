# 任务 24 · 部署与运维 FAQ（先读本目录 README.md）

## 背景

两天开发踩了大量坑：镜像/代理、onnxruntime 互杀、numpy 钉版、DML 并发死机、Roboflow 多边形标注、vite IPv6、Git Bash 中文 JSON、OpenH264 DLL、虚拟显示器不稳……这些散落在 README/部署指南/训练规程/风险登记册里。新部署者（和未来的我们）需要一份集中 FAQ。

## 目标（纯文档，不改代码）

新建 `docs/部署FAQ.md`，按主题分组，每条格式：**现象 → 原因 → 解决命令/步骤**：

1. **安装与网络**：PyPI 慢/断（清华源）、GitHub 下载失败（ghfast/断点续传）、HF 不可达（hf-mirror）、npm（npmmirror）、Roboflow 需代理（7890）与 `api_key` 参数名
2. **依赖冲突**：onnxruntime 与 onnxruntime-directml 互杀（卸载两件重装）、numpy 1.x/2.x 二进制不兼容（钉 1.26.4）、pandas 需 2.2.3、supervision 必须 0.25.1、pytorch.org 源不可用用 PyPI
3. **运行与端口**：8000/5173/1984 端口占用处理、vite 仅绑 IPv6（curl 用 [::1]）、Git Bash curl 中文 JSON 损坏（用 httpx）、Windows DLL 锁导致 pip 安装失败（先停服务）
4. **模型与推理**：DirectML 并发死机（串行锁机制解释 + 禁止 sess.run）、NPU XINT8 量化配方与 Quark 绕道、模型加载失败排查（classes.txt 顺序/路径）、PEP 阈值证据分级说明
5. **训练**：限线程训练命令、训练安全规程链接、训练中断识别（.training.lock）、CPU 死机历史与硬件侧建议（虚拟显示器/BIOS/驱动）
6. **数据与标注**：Roboflow YOLO 导出可能是分割多边形（需转外接框）、标注规范链接、数据集许可（CC BY 4.0）
7. **告警与推送**：各渠道 key 获取入口、域名白名单机制、测试告警端点、失败重发
8. **前端**：bundle 预算、主题令牌位置（theme.css）、改动禁区（主线专属文件）

## 验收标准

每条现象都能被实际复现或已经历过（不编造问题）；命令可直接复制执行；部署指南与 README 末尾加链接指向本 FAQ；不改代码。

## 执行报告

### 交付物
- `docs/部署FAQ.md`（新，八主题 30+ 条）：安装与网络 / 依赖冲突（onnxruntime 互杀、numpy 钉版、
  supervision 0.25.1、DLL 锁）/ 运行与端口（占用、vite IPv6、Git Bash 中文、OpenH264 DLL）/
  模型与推理（DML 并发死机与串行锁、NPU 量化绕道、classes.txt 契约、PPE 证据分级）/ 训练（限线程命令、
  安全规程、中断识别、GPU 路线、硬件侧建议）/ 数据与标注（分割多边形坑、规范、许可）/ 告警与推送
  （key 入口、白名单、自测、重发）/ 前端（bundle 预算、主题令牌、改动禁区、401）
- 全部条目来自本项目实际经历（roadmap/风险登记册/各任务执行报告有据可查），无编造
- 每条含可直接复制的命令；部署指南与 README 文档索引已加 FAQ 链接

### 验收结果
- 纯文档，不改代码；pytest 不受影响
- 覆盖任务书 8 个主题全部命中

### 遗留问题
- FAQ 为快照式整理，后续新踩坑按主题补录（已在文档头注明依据时间范围）
