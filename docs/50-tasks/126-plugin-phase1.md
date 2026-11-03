# 任务 126 · 插件一期：约定式外挂（P2，默认禁用）

> 来源：flash 建议“插件一期”（SPlayer/Scrypted 思路）+ 用户明确诉求：
> **未合并主线的功能先用插件外挂**。一期只做“约定目录 + 清单 + 手动/自动启停 + 状态 + 示例”，
> 不做进程沙箱/Grant UI（二期），但所有红线先立住：默认禁用、管理员启用、权限最小化、不进 git。

## 目标

1. 目录约定：`<data_dir>/plugins/<id>/plugin.json` + 插件入口（任意语言进程）。
2. `plugin.json`（pydantic 严格校验，`extra="forbid"`）：
   ```json
   {
     "id": "event-logger",
     "name": "事件日志外挂",
     "version": "0.1.0",
     "description": "订阅事件并写 CSV（示例）",
     "command": ["python", "main.py"],
     "permissions": ["read"],
     "autostart": false,
     "env": {"OPTIONAL_KEY": "value"}
   }
   ```
   - `id` 必须匹配 `^[a-z0-9][a-z0-9_-]{1,31}$`；`command` 数组、不允许 shell 字符串；
   - `permissions` ⊆ `["read","write"]`；`env` 不允许覆盖 `LS_*` 系统变量（防劫持）；
   - 入口路径必须位于插件目录内（resolve 后校验）。
3. 生命周期 API（`require_admin`）：
   - `GET /api/plugins`：清单 + enabled/running/pid/last_error/started_at/权限；
   - `POST /api/plugins/{id}/enable|disable`：持久化状态（`data/plugins/state.json`，原子写）；
   - `POST /api/plugins/{id}/start|stop`：禁用状态拒绝启动；停止先 terminate → 5s 后 kill；
   - `GET /api/plugins/{id}/logs?tail=200`：读插件日志（路径安全）。
4. 启动注入环境：`LS_PLUGIN_ID`、`LS_API_BASE=http://127.0.0.1:<port>`、
   `LS_PLUGIN_DATA=<data>/plugins/<id>/data`；令牌：
   - `permissions=["read"]` 且配置了 `LS_READ_TOKEN` → 注入 `LS_PLUGIN_TOKEN=<read_token>`；
   - 含 `write` 且配置 `LS_ADMIN_TOKEN` → 注入 `LS_PLUGIN_TOKEN=<admin_token>`（启用 UI 明确警告）；
   - 未配置鉴权 → 不注入；**绝不把 token 写进插件清单/日志**。
5. 启动策略：默认**不**自动启动；用户 enable 后可手动 start；`autostart=true` 只对已 enable 的插件生效。
   崩溃不做自动重启（二期做上限），状态显示 exited/returncode。
6. UI：Cameras 页底部“插件（外挂）”折叠区：列表/启用/启动/停止/看日志；启用未配置权限时提示。
7. 示例：`examples/plugins/event-logger/`（不在 data 内，文档指导复制）——轮询 `/api/events`
   写 CSV/或调 webhook；README 说明插件能做什么、不能做什么。
8. 文档：`docs/74-plugin-guide.md`（清单/权限/示例/安全红线/与二期差异）；README 索引。

## 实现建议

- 新 `server/app/plugins.py`：`PluginManager`（扫描/校验/状态/进程表），线程锁；启动时扫描，
  不自动启动任何插件。
- `main.py` lifespan：初始化 manager，stop_all on exit；`/api/system` 可选带插件计数。
- Windows 子进程 `CREATE_NO_WINDOW`（复用 launcher 的常量/写法）；cwd=插件目录；stdout/stderr
  重定向到 `data/plugins/<id>/logs/plugin.log`（轮转/大小上限 5MB×2 可选）。
- 插件环境从 `os.environ` 复制后覆盖白名单；清理 `LS_ADMIN_TOKEN/LS_READ_TOKEN` 防止意外继承。
- 状态文件原子写（tmp+replace）；损坏时回退“全部 disabled”并 WARNING。

## 测试（`server/tests/test_plugins.py`）

- 清单校验：合法/非法 id/command 字符串/越界 permissions/`env` 覆盖 LS_*/路径穿越 → 拒绝；
- 扫描：只读 data/plugins 下目录；不存在的目录/坏 JSON 不崩；
- enable/disable 持久化与幂等；disabled 不能 start；start 用 fake Popen 验证 env/cwd/参数，
  不真的执行命令；stop 先 terminate 再 kill（fake）；
- token：read/write 选择、无鉴权不注入、env 不泄 token；
- logs tail 路径安全；API 401 用例；
- 插件进程退出后状态/returncode 更新。
- 不执行真实第三方代码。

## 验收

- `pytest tests/ -q` 全绿；默认安装无插件、零进程、零端口；
- 报告附清单/API/示例说明；明确“插件=代码执行，默认禁用，管理员显式启用”。

## 约束

- 不引入新依赖；不做进程沙箱/Grant UI/自动重启上限（二期）；
- 不自动运行插件、不打包示例进便携包（文档指导）；
- 插件目录/状态文件 gitignore（`data/` 已忽略）。
