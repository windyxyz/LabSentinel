# 任务 43 · 单端口模式：服务端托管 web/dist（打包前置）

> 背景：现在必须开 Vite（5173）才有界面，打包版不能带 Node。让 FastAPI 直接托管 `web/dist`，一个端口就能用；这也是便携包/安装包的第一步。

## 目标

1. **配置**：`Settings` 增加 `web_dist: Path`（默认 `BASE_DIR.parent / "web" / "dist"`，env `LS_WEB_DIST` 可覆盖）。
2. **静态托管**（`server/app/main.py`，路由注册完之后）：
   - 若 `web_dist/index.html` 存在：挂载 `SPAStaticFiles` 到 `/`；
   - `SPAStaticFiles(StaticFiles)` 覆写 `get_response`：静态文件 404 时，若路径最后一段不含 `.`（不是资源文件）→ 返回 `index.html`（SPA 回退）；含扩展名仍 404，避免把缺失 js 伪装成 html。
   - 若 dist 不存在：`GET /` 返回 JSON 提示（如 `{"message": "web dist not built; run npm run build"}`），不影响 API。
3. **不破坏现有路由**：`/api/*`、`/ws/*`、`/docs`、`/openapi.json` 必须优先于静态挂载；用 TestClient 断言。
4. **文档**：`.env.example` 增加 `LS_WEB_DIST`；`docs/部署指南.md` 或 README 补一句"生产/打包模式：npm run build 后只开 8000，访问 http://127.0.0.1:8000"。
5. **测试**（`server/tests/test_web_static.py`）：
   - 临时目录造 `index.html` + `assets/app.js`；
   - `GET /` 200 且含 index 标记；`GET /events` 200 返回 index（SPA）；
   - `GET /assets/app.js` 200；`GET /assets/missing.js` 404；
   - `GET /api/system` 仍 JSON；`GET /docs` 200；无 dist 时 `GET /` 返回提示 JSON。

## 约束

- 不改服务端分析/API 业务逻辑；不动主线样式；不引入新依赖（用 Starlette StaticFiles）。
- 测试用临时 dist，不依赖本机 `web/dist` 实际构建；`pytest` 全绿。
- 不做打包/启动器（任务 44/45）。

## 验收标准

- 单端口启动（8001）后浏览器直接打开 UI，无需 Vite；刷新 `/events` 不 404；
- API/WS/Docs 正常；缺 dist 时给出明确提示；
- 新增测试通过，pytest 总数增加且全绿；
- 文档/env 示例更新。

## 执行报告

（待填：改动文件 / 新测试 / 单端口实测 / 兼容性检查 / 遗留问题）

### 执行报告（flash，2026-10-28 01:18 交付）

- **改动文件**：
  - `server/app/config.py`：+`web_dist: Path = BASE_DIR.parent/"web"/"dist"`（env `LS_WEB_DIST` 自动映射）。
  - `server/app/main.py`：`SPAStaticFiles(StaticFiles)` 覆写 `get_response`——404 且末段无扩展名 →
    回 index.html（SPA 回退），带扩展名仍 404；`mount_web(app)` 可重入挂载（先移除 name=web_dist
    的旧路由再挂），dist 缺失时 `GET /` 返回提示 JSON；import 时调用一次，位于 include_router 与
    `/ws/events` 之后保证 API/WS/Docs 优先。
  - `server/tests/test_web_static.py`（新，6 用例）：/ 出 index、/events SPA 回退、/assets/app.js 200、
    /assets/missing.js 404、/api/system JSON 且 /docs 200（优先级）、缺 dist 提示 JSON。
  - `.env.example`（+LS_WEB_DIST 段）、`docs/部署指南.md`（部署形态节加单端口说明）。
- **坑与修复**：StaticFiles 抛的是 **starlette** 的 HTTPException，用 fastapi 的子类 except 接不住
  （SPA 回退首次全 404）——改从 starlette.exceptions 导入后 6/6 过。
- **验证**：`pytest tests -q` **84 passed**（78+6）；单端口 8001 实测：`/` 200 html、`/events` 刷新
  200（回 index）、`/assets/missing.js` 404、`/api/system` 200 JSON、`/docs` 200；收尾 8000/8001/5173 全空闲。
- **遗留问题**：无。任务 44（便携运行时）可直接依赖本能力（不再带 Node/Vite）。
