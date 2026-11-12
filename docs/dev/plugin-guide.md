# 插件（外挂）指南 · 一期（任务126）

> 一句话：**插件=代码执行**——默认禁用、管理员显式启用、权限最小化。
> 用途：把"未合并主线的功能"先用外挂实现（自定义报表/联动/转发），不动主服务。
> 与 MQTT 总线（[73 协议](mqtt-protocol.md)）的关系：总线是"事件出去"，插件是"逻辑外挂"。

## 1. 目录与清单

```
<数据目录>/plugins/<id>/          # 便携版在 %LOCALAPPDATA%\LabSentinel\data\plugins\
  plugin.json                     # 清单（严格校验）
  main.py / main.exe / ...        # 入口（任意语言，命令行可启动即可）
  data/                           # 宿主注入 LS_PLUGIN_DATA 指向这里（插件私有）
  logs/plugin.log                 # 宿主重定向的 stdout/stderr（「看日志」按钮读这里）
```

`plugin.json`：

```json
{
  "id": "event-logger",
  "name": "事件日志外挂（示例）",
  "version": "0.1.0",
  "description": "轮询 /api/events 写 CSV",
  "command": ["{python}", "main.py"],
  "permissions": ["read"],
  "autostart": false,
  "env": {"POLL_SECONDS": "10"}
}
```

校验规则（违规拒绝加载）：`id` 匹配 `^[a-z0-9][a-z0-9_-]{1,31}$` 且=目录名；`command`
必须非空字符串数组（**不接受 shell 字符串**）；`permissions` ⊆ `read`/`write`；
`env` **不允许覆盖 `LS_*`** 系统变量；未知字段拒绝（extra=forbid）。

`command[0]` 解析顺序：`{python}`（宿主当前解释器，推荐）→ PATH 上的裸命令（如 `python`）
→ 插件目录内相对路径；**目录外绝对路径一律拒绝**（`..` 逃逸 resolve 后也必须仍在插件目录内）。

## 2. 宿主注入的环境

| 变量 | 值 |
|---|---|
| `LS_PLUGIN_ID` | 插件 id |
| `LS_API_BASE` | `LS_PUBLIC_BASE_URL`（缺省 `http://127.0.0.1:8000`） |
| `LS_PLUGIN_DATA` | 插件私有数据目录 |
| `LS_PLUGIN_TOKEN` | 按权限注入（见下），**绝不写进清单/日志** |

令牌规则：`read` 权限 + 服务端开读保护 → 注入 `read_token`；含 `write` 权限 → 注入
`admin_token`（**启用含 write 的插件 = 把管理员权限交给它，UI 明确警示**）；未配置鉴权
不注入。子进程环境会先清除宿主的 `LS_ADMIN_TOKEN/LS_READ_TOKEN` 防意外继承。

## 3. 生命周期（当前为管理 API；Web UI 折叠区在任务 128）

- `enable`/`disable`：持久化到 `state.json`（原子写；损坏回退全禁用）；
- `start`/`stop`：仅 enabled 可启动；停止先 terminate 5s 宽限再 kill；
  崩溃**不自动重启**（二期做上限），状态列显示 returncode；
- `autostart=true` 只对已 enable 的插件在服务启动时生效；
- 日志：`GET /api/plugins/{id}/logs?tail=200` 读 `logs/plugin.log` 末尾。

```bash
curl -X POST http://127.0.0.1:8000/api/plugins/event-logger/enable -H "X-Admin-Token: ..."
curl -X POST http://127.0.0.1:8000/api/plugins/event-logger/start  -H "X-Admin-Token: ..."
curl http://127.0.0.1:8000/api/plugins -H "X-Admin-Token: ..."
```

## 4. 示例：event-logger

`Resources/examples/plugins/event-logger/`（不在数据目录内，防误启）。安装：

1. 复制整个目录到 `<数据目录>/plugins/event-logger/`；
2. 用上面的 API **启用** → **启动**（Web UI 随任务 128 提供）；
3. `GET /api/plugins/event-logger/logs` 确认轮询，`data/` 下生成 `events.csv`。

它演示了插件能做什么（轮询 REST 写文件/调外部 webhook/生成报表）；**不能做什么**：
不能改事件处置状态以外的数据库、不能开端口、不能加载模型——一期不给插件任何宿主内
API，只给 HTTP + 令牌（进程隔离天然是边界）。

## 5. 安全红线（一期即生效）

1. 默认禁用：新装的插件不出现在任何自动执行路径；
2. 管理员显式 enable；disable 会连运行中的进程一起停；
3. 权限最小化 + 令牌只进子进程环境；
4. 入口必须在插件目录内（解释器可用 `{python}`/PATH 命令；目录外绝对路径拒绝）；
5. 插件代码不进 git（`data/` 已忽略；示例在 `Resources/examples/` 且不进便携包）。

## 6. 与二期（规划）的差异

二期计划：子进程沙箱加固（每插件独立进程+资源上限）、Grant 权限确认 UI、崩溃自动重启
上限、manifest 版本化与目录哈希校验。一期 API 形态保持兼容（只加不减）。
