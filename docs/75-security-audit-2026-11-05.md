# 安全与依赖审计（任务134，2026-11-04 flash）

> 范围：批次 122-130 新增面（paho-mqtt/插件子进程/VLM 外联/WS-Discovery/PWA SW/单事件
> 证据包/守护/分组）。方法：依赖审计工具实测 + 源码逐项核查 + 修复回归测试。
> 结论标记：PASS｜WARN（已修）｜ACCEPT（有理由接受）。

## 一、依赖

| 项 | 工具 | 结论 | 说明 |
|---|---|---|---|
| Python 依赖 | `pip-audit -r server/requirements.txt` | **PASS**：No known vulnerabilities found | 含新依赖 paho-mqtt>=1.6,<2 |
| npm 依赖 | `npm audit --omit=dev` | **ACCEPT**：审计端点被网络策略拦截（registry.npmjs.org 不可达） | 生产依赖仅 vue@^3.4；devDeps（vitest/jsdom/test-utils）不进构建产物；留待有网环境 CI 补跑 |

## 二、路径/文件

| # | 项 | 结论 | 证据 |
|---|---|---|---|
| 1 | 插件入口目录校验 | PASS | 主线复审已改 `Path.is_relative_to`；回归 `test_start_entry_sibling_prefix_rejected`（`../event-logger-evil` 同前缀兄弟目录拒绝） |
| 2 | 插件日志/状态路径 | PASS | `logs()` 双重防护（id 正则 + resolve 父目录校验）；state.json 原子写 |
| 3 | 单事件/批量证据包 | PASS | 76 `_within_data_dir` 严格包含；`..`/跨盘/data 外跳过（`test_event_evidence` 覆盖）；zip 成员名由代码生成（非用户输入），无 zip-slip |
| 4 | folder 图片源 | PASS | 创建时目录存在性+data 目录自激校验（api.py folder 分支）；扫描走目录枚举不拼用户路径 |
| 5 | 配置导入**规模上限** | **WARN→已修** | 原无上限；新增摄像头≤200/区域≤1000/规则≤2000（400 拒绝）。**修复过程抓到真 bug**：cap 原写在填充循环后且对空列表判断——已移到循环前直量原始输入；`test_config_import_scale_cap` 回归 |
| 6 | 人脸照片上传**大小上限** | **WARN→已修** | 原无上限（`photo.read()` 全量入内存）；新增 10MB 上限（413）；`test_face_photo_10mb_cap` 回归 |

## 三、外联/SSRF

| # | 项 | 结论 | 证据 |
|---|---|---|---|
| 1 | GenAI URL | PASS | 回环白名单（127.0.0.1/localhost/::1），非回环需 `LS_GENAI_ALLOW_REMOTE`（发请求前校验） |
| 2 | 更新检查 | PASS | 协议白名单（https 或显式 allow_http 的 http，先查后连）；`follow_redirects=False`（测试断言）；成功才缓存；不访问清单里的 url 字段 |
| 3 | MQTT host | PASS | 只连配置 host:port；paho 自动重连；broker 不可达不影响主链路（实机验证） |
| 4 | Webhook | PASS | 默认仅 https（http 需显式放行）、拒绝 user:pass@ 凭证、HMAC 签名；域名不可枚举故不设白名单（设计取舍，文档已写） |
| 5 | 发现助手 | PASS | `is_lan` 白名单（10/8、172.16/12、192.168/16、169.254/16），公网/回环全滤；probe 同白名单；XML 拒 DOCTYPE/ENTITY（XXE）；公网 IP 过滤有测试 |
| 6 | 告警渠道域名 | PASS | 渠道 URL 由管理员配置（非用户输入）；发送前 `redact_url` 脱敏日志 |

## 四、令牌/凭据

| # | 项 | 结论 | 证据 |
|---|---|---|---|
| 1 | plugin token 不回显 | PASS | `test_token_injection_rules` 断言 `adm-token not in json.dumps(scan())`；UI 只显示权限徽章不显示令牌 |
| 2 | 插件子进程环境清理 | PASS | `_build_env` 剥离 `LS_ADMIN_TOKEN/LS_READ_TOKEN` 后按权限注入（测试断言） |
| 3 | MQTT_PASSWORD/GENAI_API_KEY/SMTP 脱敏 | PASS | `redact.SENSITIVE` 正则（TOKEN/PASSWORD/SECRET/KEY/PASS）全部命中；`test_redact_covers_mqtt_password` 断言 |
| 4 | 诊断包 | PASS | 经同一 redact 管道；grep 测试样例无明文 |
| 5 | PWA SW 缓存范围 | PASS | sw.js 明确排除 `/api/*`、`/ws`（`test sw_no_api_cache`）；仅缓存 `/`、index、assets（hash 文件）、图标 |

## 五、插件执行

| # | 项 | 结论 |
|---|---|---|
| 1 | 默认禁用 | PASS（新装零插件零进程；disabled 拒启有测试） |
| 2 | Popen 无 shell | PASS（`command` 必须数组，manifest 校验拒 shell 字符串；Popen 直接传 argv） |
| 3 | 权限说明 | PASS（UI write 红色徽章+启用 confirm 明示"获得管理员令牌"） |
| 4 | 日志不泄 token | PASS（token 只进子进程环境变量；日志是插件 stdout/stderr 重定向） |

## 六、CSP/安全头/PWA 其它

| # | 项 | 结论 |
|---|---|---|
| 1 | 更新检查 URL 仅展示 | PASS（前端 `<a target=_blank rel=noopener>`，无自动打开/下载） |
| 2 | 安全头默认开 | PASS（129 版 release_check 全 PASS；csp 默认空可选） |
| 3 | OpenAPI 快照 | PASS（示例全演示数据；`test_openapi` 断言无密钥键名） |

## 七、修复清单（本任务）

1. **配置导入规模上限**（api.py `_validate_config_import`）+ `test_config_import_scale_cap`；
   修复中发现的真缺陷：cap 判断位置错误（对空列表判断，永不触发）——字节码反汇编定位，
   移到填充循环前直量原始输入。
2. **人脸照片 10MB 上限**（413）+ `test_face_photo_10mb_cap`。
3. OpenAPI 快照过期（132 加参后）重生成 + `test_openapi` 恢复绿（顺带修复）。

## 八、遗留/接受

- **ACCEPT**：npm audit 网络拦截（生产依赖面极窄：vue 单运行时依赖；CI 有网环境可补跑）；
- **ACCEPT**：MQTT/GenAI/发现助手均默认关闭或内网限定，攻击面在"管理员显式开启"之后；
- 无 FAIL 未处置项；无 BLOCKED。
