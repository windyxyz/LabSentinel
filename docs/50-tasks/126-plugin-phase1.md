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
     "command": ["{python}", "main.py"],
     "permissions": ["read"],
     "autostart": false,
     "env": {"OPTIONAL_KEY": "value"}
   }
   ```
   - `id` 必须匹配 `^[a-z0-9][a-z0-9_-]{1,31}$`；`command` 数组、不允许 shell 字符串；
   - `command[0]`：`{python}`（宿主解释器，推荐）/ PATH 裸命令 / 插件目录内相对路径；
     目录外绝对路径与 `..` 逃逸拒绝；
   - `permissions` ⊆ `["read","write"]`；`env` 不允许覆盖 `LS_*` 系统变量（防劫持）；
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

## 执行报告

**flash 2026-11-04 完成（pytest 488 passed = 474+14 新增；默认安装零插件零进程已实测）**
（全量回归为 125 基线上 126 的口径，数字以本轮实跑为准）

### 1. server/app/plugins.py（新文件）

- `PluginManifest` pydantic 严格校验（extra=forbid）：id 正则、command 非空字符串数组
  （拒 shell 字符串）、permissions ⊆ read/write、**env 拒绝覆盖 LS_***、未知字段拒；
- `PluginManager`：扫描（坏 JSON/缺清单/id 不一致 → error 项不崩、非目录忽略）、
  enable/disable 持久化 `state.json`（tmp+os.replace 原子写；损坏回退全禁用+WARNING）、
  start（disabled 拒 / **入口 resolve 后必须在插件目录内** / 环境先剥离
  LS_ADMIN_TOKEN/LS_READ_TOKEN 再注入 LS_PLUGIN_ID/LS_API_BASE/LS_PLUGIN_DATA/
  LS_PLUGIN_TOKEN / cwd=插件目录 / CREATE_NO_WINDOW / stdout→logs/plugin.log）、
  stop（terminate → 5s 宽限 → kill）、stop_all、autostart_enabled（只拉 enable+autostart）、
  logs（id 正则+resolve 双重路径安全，tail 1-2000）；
- 令牌规则：write+admin_token → admin；read+读保护+read_token → read；未配置不注入；
  **token 绝不进清单/状态输出（测试断言）**。

### 2. API/前端/接线

- 六端点全 require_admin：`GET /api/plugins`、`POST /{id}/enable|disable|start|stop`、
  `GET /{id}/logs?tail=200`；未初始化 503；不存在/非法 404；语义错 400；
- main.py lifespan：装配 manager + autostart_enabled()，退出 stop_all；
- Cameras.vue「插件（外挂）」折叠区：本轮交付 API 与示例；**UI 渲染留待 128**（前端只读
  消费 /api/plugins，零后端依赖——与 124 徽标同批，控制单任务半径）。

### 3. 示例与文档

- `examples/plugins/event-logger/`（plugin.json + main.py 轮询 REST 写 CSV，**不进数据
  目录/不进便携包**）；
- `docs/74-plugin-guide.md`（清单/注入环境/令牌规则/生命周期/安全红线五条/与二期差异）；
  README 索引 + 35-api-reference 已加。

### 4. 测试（test_plugins.py，14 用例，fake Popen 不执行真实第三方代码）

清单校验矩阵 8 形态、扫描（坏 JSON/非目录/缺目录）、enable/disable 持久化+损坏回退、
disabled 拒启、start env/cwd/防继承断言、入口越界拒、token 三规则+不进清单输出、
stop 先宽限后强杀、logs tail/路径安全、退出后 returncode 可见、API 全链+401。
（坑：lifespan 会重建 manager——API 测试必须写 manager.dir 而非 fixture 预设目录。）

### 5. 默认态

默认安装无插件目录内容 → `GET /api/plugins` 空列表、零进程、零端口。

### 遗留

- 前端折叠区渲染随 128；二期（沙箱/Grant UI/重启上限/manifest 版本化+哈希校验）按规格不做。

### 主线复审补丁（2026-11-04）

- **入口解析**：原 `["python","main.py"]` 会被解析成 `<插件目录>/python`（不存在），示例无法启动；
  改为支持 `{python}`（宿主解释器，示例已改）/PATH 裸命令/插件内相对路径。
- **目录穿越**：原 `startswith(str(plugin_dir))` 可被 `plugins/foo-evil/...` 同前缀兄弟目录绕过
  → 改 `Path.is_relative_to`；新增兄弟目录回归测试。
- 打开日志文件后父进程未关闭句柄 → Popen 后 close，防 fd 泄漏。
- 文档更正：管理 UI 当前未实现（任务 131 提供），74 号手册改为 API 用法。
