# 任务 117 · 版本更新检查（P2，不自动安装）

> 背景：便携版靠用户手动下载新 zip；希望面板能提示“有新版本”。本任务只做**检查与提示**，
> 不自动下载/安装（无托管渠道、签名未定，自动更新风险高）。

## 目标

### 1. 配置

- `LS_UPDATE_MANIFEST_URL`：版本清单 URL（默认空=关闭）；
- `LS_UPDATE_ALLOW_HTTP=0`：默认仅 https（自建局域网可显式开）；
- `LS_UPDATE_TIMEOUT_S=5`。

清单格式（自建静态 JSON）：
```json
{"version":"2026.11","url":"https://.../LabSentinel-portable-20261101.zip",
 "sha256":"...","notes":"...","published_at":"2026-11-01"}
```

### 2. API

- `GET /api/update/check?refresh=0`（require_read）：
  - 未配置 → `{enabled:false}`；
  - 配置 → httpx GET（不跟随重定向、超时、仅 https 除非 allow_http）；
  - 比较清单 version 与当前版本（`app/VERSION` 或 `_system_version`）：字符串不等即 `update_available:true`
    （不做 semver 排序，文档说明）；
  - 返回 `{enabled,current,latest,update_available,url,sha256,notes,published_at,error}`；
  - 结果缓存 1h（`refresh=1` 强制）；网络失败返回 `error` 不 500；
  - **不下载/不执行**任何 URL。

### 3. 前端 `Dashboard.vue`

- 顶部/底部小卡片“版本”：当前版本；有更新时显示“发现新版本 2026.11” + 「查看」链接
  （新窗口打开清单 url）+ 「忽略此版本」（localStorage）；
- 未配置/检查失败不打扰（不显示红条）；手动「检查更新」按钮；
- 不新增路由/不改全局四件套。

### 4. 测试 `server/tests/test_update_check.py`

- 未配置 enabled=false；https 清单解析/版本不同/相同；
- http 默认拒绝、allow_http 通过；超时/非 JSON/缺字段 → error 非 500；
- 缓存：连续两次只请求一次，refresh=1 再请求；
- 不下载 URL（mock 断言只 GET manifest）；
- 鉴权口径。

### 5. 文档

- `.env.example` 3 项；`部署FAQ`「怎么配更新提示/为什么不在线升级」；
- `运维手册` 升级一节加“手动下载 zip→备份→覆盖升级”。

## 约束

- 不自动下载/安装/执行；不跟随重定向；不发送本机信息（仅 GET manifest）；
- 不引入依赖；测试 mock httpx，不访问外网；
- 前端只改 Dashboard.vue/api.js。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 8001 用本地 HTTPS/HTTP mock 清单实测 available/相同/失败三态；
- 报告附响应 JSON（脱敏）。

## 执行报告

**flash 2026-11-04 完成（pytest 445 passed = 436+9 新增；build gzip 59.97KB；三态实机 PASS）**
（全量回归为 116 seed 修复后基线上 117 的口径；结果见文末）

### 1. 配置（config.py + .env.example）

- `update_manifest_url=""`（空=关闭）、`update_allow_http=False`、`update_timeout_s=5.0`；
- 清单格式含可选 `channel`（**SPlayer-Next #7 折叠项**：stable/beta 通道随清单回显，
  Dashboard 显示"发现新版本 X（stable）"）。

### 2. server/app/update_check.py（新文件）

- `fetch_manifest`：**协议白名单（https 或显式 allow_http 的 http，检查在发请求前）→
  `follow_redirects=False`（不跟重定向）→ 200 → JSON → version 字段**；任何失败抛
  `UpdateCheckError`（可读信息）；
- `check_update`：1h 缓存（**只缓存成功结果**——失败不粘住，下次可重试，实现期单测抓到
  错误结果被缓存的问题已修）；`refresh=1` 强制；version 与当前**字符串不等即提示**；
  只回显清单字段（url/sha256/notes/published_at/channel），**绝不访问 url/下载/执行**。

### 3. API/前端

- `GET /api/update/check?refresh=0`（require_read）→ `{enabled,current,latest,
  update_available,url,sha256,notes,published_at,channel,error}`；未配置 `{enabled:false}`；
- Dashboard「版本」卡：当前版本 +「检查更新」按钮；有更新显示"发现新版本 X（channel）"+
  「查看」（新窗口开 url）+「忽略此版本」（localStorage `LS_UPDATE_IGNORE`）；
  未配置/失败只静默显示当前版本（不打扰红条）；api.js `updateCheck`。

### 4. 测试（test_update_check.py，9 用例，mock httpx 零外网）

未配置 enabled=false、https 清单解析+版本不同/相同、http 默认拒（**协议检查先于请求，calls==[]**）/
显式放行、**follow_redirects=False 断言**、超时/非 JSON/缺 version/404 → error 不 500、
缓存 1h+refresh 强制、**只 GET manifest 一次**（url 字段不被访问）、API 流程与鉴权 401。
**坑记录**：bytes 字面量不能含中文（清单样例改 str.encode）。

### 5. 实机验证（8190 本地清单服务器 + 三个独立 8188/8189/8191 实例）

- available：`(True, '2026.11', '')`；same：`(False, '0.1.0-m1', '')`；
  bad：`(False, None, '清单不是合法 JSON')` → **三态 PASS**；
- manifest 恰好被 GET 3 次（每实例一次；缓存生效、零下载访问）。

### 6. 文档

.env.example 3 项；部署FAQ「怎么配更新提示/为什么不在线升级」（含清单格式与手动升级链）；
运维手册 §8 升级一节加更新提示说明。

### 约束遵守

不自动下载/安装/执行；不跟重定向；只 GET 清单不带本机信息；零新依赖（httpx 已有）；
测试全 mock；前端只改 Dashboard.vue/api.js。

### 遗留

- 无。
