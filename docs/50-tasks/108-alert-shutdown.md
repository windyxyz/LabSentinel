# 任务 108 · 告警投递线程池退出与自测并发（P2）

> 来源：flash 审阅建议 #3。dispatcher 的 ThreadPoolExecutor 非 daemon，进程退出可能被重试拖住数十秒；
> `send_test` 串行逐渠道发送，最坏 ~50s（每渠道 5s 超时 × 8）。

## 目标

1. 退出：
   - lifespan finally 调用 `dispatcher.shutdown(wait=False, timeout=3)`（新增方法）；
   - 关闭时取消未开始的 future；正在重试的线程最多等 3s，不阻塞服务退出；
   - 若 Python 版本允许，executor 线程设为 daemon（自定义线程工厂）；不允许则记录说明。
2. `send_test`：
   - 对所有已配置渠道并发提交到短生命周期 executor（或现有池），每个 5s 超时；
   - 总耗时 ≤ max(单渠道超时)+1s（约 6s），返回每渠道结果；
   - 未配置渠道仍返回「未配置」且不建投递行；
   - 结果顺序稳定（按渠道列表），不因并发乱序。
3. 测试 `server/tests/test_dispatcher_shutdown.py`：
   - 假慢渠道（sleep 2s）→ send_test 总耗时 <4s（用 2 个渠道验证并发）；
   - shutdown(wait=False) 在 1s 内返回；不抛异常；
   - 未配置渠道不投递（回归）。

## 约束

- 不改告警内容/重试策略；不引入依赖；
- 不能让并发自测产生重复投递记录（自测本就不写库）；
- 测试不访问网络（stub 渠道）。

## 验收

- `pytest tests/ -q` 全绿；报告附耗时对比（串行 vs 并发）；
- 退出服务实测不被投递线程拖住（可用假渠道模拟）。

## 执行报告

（待填：改动 / 耗时 / 测试 / 遗留）
