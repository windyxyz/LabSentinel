# 任务 22 · 端到端冒烟脚本（先读本目录 README.md）

## 背景

单元测试（67 passed）覆盖模块逻辑，但没有"起服务→配摄像头→画区→加规则→触发事件→校验→清理"的端到端自动化。发布前每次改动都需要人工回归，冒烟脚本可让 CI/发布前一条命令验证主链路。

## 目标

新建 `tools/e2e_smoke.py`（python，httpx，**不启动/停止服务进程**——假设服务已在运行；不可用时提示先启动）：

1. 前置检查：`GET /api/system` 可达，否则清晰报错退出
2. 注册文件摄像头（autostart=false，源用 `m0-spike/data/lab-walk-24076.mp4` 绝对路径）
3. 创建安全区（多边形同演示配置 [[520,180],[1180,180],[1180,620],[520,620]]）
4. 添加规则：enter（L2）、enter_no_ppe（L3）
5. 启动摄像头 → 轮询 `/api/events?camera_id=` 最多 40 秒，等待 ≥1 条 enter 事件
6. 校验：事件字段完整（ts/type/level/detail/has_snapshot）；进入事件成对（enter 存在）
7. 断言 `GET /api/events/{id}/snapshot` 返回 image/jpeg；`GET /api/events/count` 与 events 行数一致
8. 清理：停止/删除测试摄像头（连带 zone/rule），打印 PASS/FAIL 摘要表；退出码 0/1

## 约束

- 全程走 API（不 import server 内部模块，保持黑盒）
- 不占 8000/5173 以外端口；不写生产数据目录以外的文件
- 支持 `--base-url`（默认 http://127.0.0.1:8000）、`--timeout`、`--keep`（调试时保留测试数据）
- 不触发 AI 训练/压测

## 验收标准

在服务运行且空闲时实跑一次输出 PASS 表与退出码 0；服务未启动时给出明确错误；脚本不进 pytest（独立工具）；执行报告附一次实跑输出摘要。

## 执行报告

### 交付物
- `tools/e2e_smoke.py`（黑盒 httpx，不 import 服务端模块，不启停服务进程）：
  ①前置检查（不可达时清晰报错+退出码1）→ ②注册文件摄像头（autostart=false）→ ③安全区
  （规格指定多边形）→ ④enter(L2)/enter_no_ppe(L3) 规则 → ⑤启动+轮询（默认 40s，等待 ≥1 条
  enter 事件，DirectML 初始化留量）→ ⑥字段完整性+enter 成对校验 → ⑦截图 image/jpeg 断言 +
  events/count 一致性 → ⑧清理（删摄像头连带安全区/规则；--keep 保留调试）
- 参数：--base-url / --timeout / --keep；PASS/FAIL 摘要表；退出码 0/1
- 独立工具，不进 pytest；文件头注明"训练/压测期间禁跑"（触发 DML 推理，重负载铁律）

### 验收状态
- `py_compile` / `--help` / **服务未启动路径**（连 9999 端口 → "[FAIL] 服务不可达——请先启动服务"）实跑通过
- **完整实跑（PASS 表 + 退出码 0）待训练结束窗口执行**：脚本触发真实 DML 推理，当前训练满核
  （CPU 76%），立即实跑违反"重负载一次只跑一个"铁律——已列入训后待办（训练完成链路完成后执行）
