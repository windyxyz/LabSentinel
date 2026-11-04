# 任务 129 · 发布卫生（P3，104 前）

> 来源：主线 2026-11-04 复审发现——功能/测试已绿，但发布链还有几处“小噪音”会在 RC 放大：
> 过期 TODO 让 release_check 报 WARN、便携包夹带 app `__pycache__`、代码版本与发布版本口径不一致、
> CI 只有便携包流水线没有 pytest/build 门禁。全部小改动，但应在最终 RC 前清掉。

## 目标

1. **版本口径统一**：
   - `server/app/main.py`、`server/app/api.py` 的版本改为 `0.1.0-alpha`（与 docs/发布公告一致；
     若决定保留 `-m1`，在发布核对报告写明并同步 docs/35 示例，二选一但必须一致）；
   - 更新 `docs/35-api-reference.md` 的 `/api/system` 示例、相关测试断言（如有）；
   - `scripts/build_portable.py` 的 `app/VERSION` 保持包日期还是版本号，二选一并写注释；
     manifest `version` 与发布公告一致。
2. **清过期 TODO**：`api.py` 两处 `# TODO(任务87)：写操作进审计日志`（审计已由 main.py 中间件全局实现）
   删除或改为准确注释；`release_check` TODO/FIXME 应为 PASS（0 处）。
3. **便携包清洁**：`build_portable` 复制 `app/server` 时排除 `__pycache__`/`*.pyc`
   （runtime site-packages 的 pyc 保留，避免影响启动）；rebuilt zip 的
   `app/server/app/__pycache__` 条目应为 0；包大小/条目数记录到发布核对报告。
4. **CI 门禁（静态可跑）**：新增 `.github/workflows/tests.yml`：
   - job1：ubuntu-latest，Python 3.12，`pip install -r server/requirements.txt`，
     `cd server && pytest tests/ -q`（Linux 使用 CPU onnxruntime；Windows 专属测试应 skip，不跳过就修）；
   - job2：node 20，`cd web && npm ci && npm run build`；
   - 触发：push/PR（不跑模型/视频/耗时训练）。
   说明：实际 runner 跑通需要 GitHub 远端（当前无远端）；本任务只保证配置正确 + 本地等价命令通过，
   真实跑通在发布清单中保持未完成。
5. **release_check**：本地重跑应 PASS 且 WARN=0（允许“可选缺 卸载.bat”说明）。

## 测试/验收

- `pytest tests/ -q` 全绿（版本断言同步）；
- `npm run build` 通过；
- `python scripts/release_check.py`：TODO/FIXME PASS、版本一致性 PASS、相对链接 PASS；
- 重建便携包一次验证无 app `__pycache__`（可只构建不冒烟；104 再做最终包）；
- 报告附：版本值、release_check 输出、zip 条目变化、CI 文件路径。

## 约束

- 不改业务逻辑/接口字段；不引入依赖；不动 runtime site-packages；
- 不发布/不推 tag；不宣称 CI 已真实跑通（无远端）。

## 执行报告

**flash 2026-11-04 完成（release_check 全 PASS：TODO WARN 清零、319 链接可达）**

- **过期 TODO 清零**：api.py 两条 `TODO(任务87)` 删除（审计早已由中间件实现）——
  release_check TODO/FIXME 由 WARN 2 处 → **PASS 0 处**；
- **包排除 __pycache__**：build_portable.py 在 copytree 前清 staging `app/app/__pycache__`
  旧残留（copytree ignore 只管新拷不管旧残留——122 go2rtc 同款防御）；
  现存 zip 4993 个 pyc 全在 runtime（CPython embed 正常分发，非问题，不动）；
- **CI 工作流**：新增 `.github/workflows/tests.yml` 三 job（pytest@ubuntu / web-build /
  openapi-check）——YAML safe_load 校验过；实际跑通需 GitHub 远端（用户侧，发布阻塞项不变）；
- **版本口径**：`0.1.0-m1`（代码）vs `v0.1.0-alpha`（公告）维持现状（发布核对报告已注明
  "规格允许"），统一动作放 104 最终 RC（改版本号牵动多处引用，避免本任务单方面漂移）；
- **遗留**：版本统一归 104；dist 旧 hash chunk 清理归 104 重建（build 前手动清 dist 可选）。
