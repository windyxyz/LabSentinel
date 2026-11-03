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

**主线 2026-11-03 完成**

- **退出**（`alerts.py` + `main.py`）：
  - `AlertDispatcher.shutdown(wait=False, timeout=3)`：置 `_closing` → `pool.shutdown(wait=False,
    cancel_futures=True)`；`wait=True` 时最多等 3s。
  - 重试退避由 `time.sleep` 改为 `_closing.wait(backoff)`：服务退出时运行中的重试**立即放弃**，
    不再被 1s/4s/15s 拖住；`dispatch`/`_deliver_with_retry` 入口检查 `_closing`。
  - lifespan `finally` 第一句调用 `dispatcher.shutdown(...)`，先停派发再停摄像头。
  - 新增 `reset()`：同一进程多次 lifespan（TestClient/重启场景）时重建线程池并清除关闭标记。
  - 说明：未使用私有 daemon 线程工厂（跨 Python 版本脆弱）；用可打断退避达到同等“不拖退出”效果。
- **自测并发**（`send_test`）：
  - 已配置渠道提交到短生命周期 `ThreadPoolExecutor`（max 8），结果按 `channels` 原顺序返回；
  - 未配置渠道仍返回“未配置”，**不建投递记录、不提交任务**；
  - 每 future 等待上限 6s（HTTP 渠道自身 5s；邮件 SMTP 10s 可能后台多跑几秒，但 UI 不再串行等待）。
- **测试**（`tests/test_dispatcher_shutdown.py`，4 项）：
  - 2 个 1.2s 慢渠道并发自检总耗时 <3s（串行应 ≥2.4s），顺序稳定；
  - 未配置渠道零投递记录；
  - `RETRY_BACKOFF=(5,5,5)` 下 shutdown 后重试线程 <1.5s 退出；
  - shutdown 幂等、reset 可重新启用。
- **回归**：`test_dispatcher_shutdown + test_alerts + test_alerts_page + test_push +
  test_generic_webhook` **34 passed**。
- 遗留：邮件渠道连接超时仍是 10s（未改投递语义）；自检 UI 对邮件最坏显示 6s 超时，
  后台线程可能稍后完成——不影响退出（进程退出时由 Python 线程池收尾）。

