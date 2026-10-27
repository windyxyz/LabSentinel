# 任务 36 · e2e_smoke 完整实跑 + DML v2 冒烟（任务 22 收尾）

> 背景：`tools/e2e_smoke.py` 是黑盒全链路冒烟（摄像头→安全区→规则→启动→事件→截图/计数），10-27 只验证过脚本本身与"服务不可达"路径；**完整实跑一直没做**（当时与训练互斥）。v2 模型接入后也没有一次 DML 下的端到端记录。

## 目标

1. **前置检查**：`m0-spike/.training.lock` 不存在；无训练/压测进程；8001 端口空闲；CPU 负载 <50%（高于则只做文档核对并写"顺延"）。
2. **在 8001 起服务**（绝不动 8000/5173）：
   ```bat
   cd /d D:\User\Desktop\TOOLS\监控识别\server
   D:\LabSentinel\m0-venv\Scripts\python.exe -m uvicorn app.main:app --port 8001
   ```
   后台/另开窗口运行，日志重定向到 `logs/e2e-server.log`；轮询 `/api/system` 就绪（DirectML 初始化约十几秒）。
3. **完整实跑**：
   ```bat
   cd /d D:\User\Desktop\TOOLS\监控识别
   D:\LabSentinel\m0-venv\Scripts\python.exe tools\e2e_smoke.py --base-url http://127.0.0.1:8001 --timeout 60
   ```
   - 必须看到 `===== 结果：PASS =====` 且每一步 PASS；
   - 记录 `backend=directml`、事件等待耗时、快照/计数断言结果；
   - 验证结束后脚本会自行清理测试摄像头/安全区/规则（不要加 `--keep`）。
4. **DML v2 冒烟补充**（e2e 已覆盖大部分）：
   - 事件所用模型为 v2：确认 `/api/system` 的 backend，事件 detail 不应为空；
   - 若 e2e 的事件类型只有 `enter`（无 `enter_no_ppe`），说明该帧 PPE 证据不足（`unknown`），属预期，记录即可，不要为通过而调阈值。
5. **收尾**：停掉 8001 的服务进程；确认 8000/5173 未被占用；`git status` 不应留下测试数据（e2e 清理 + server/data 本就忽略）。
6. 写执行报告：命令、关键输出（PASS 摘要）、耗时、遇到的异常与处理。

## 约束

- 严禁与训练/压测/其它推理并发；一次只跑一个重任务；
- 只占用 8001；不启动 Web；不改服务端代码与阈值；
- 不用 `--keep` 留脏数据；如中途失败，先清理再重试一次，仍失败则记录 FAIL 并停手。

## 验收标准

- `e2e_smoke.py` 退出码 0，输出 `结果：PASS`；
- 报告含完整 PASS 摘要与 backend 信息；
- 8001 已停止、无残留测试摄像头/区域/规则；
- 若因环境阻塞未跑（如 DML 初始化失败），报告必须明确写 FAIL/阻塞原因，不得伪造。

## 执行报告

（待填：前置状态 / 启动与就绪耗时 / 冒烟 PASS 摘要 / 清理确认 / 遗留问题）
