# 任务 134 · 安全与依赖审计（P2）

> 批次 122–130 新增了 paho-mqtt、插件子进程、VLM 外联、WS-Discovery、PWA service worker、
> 单事件证据包等面。需要一次集中审计，避免发布候选带着“新功能=新攻击面”的盲区。

## 范围与检查项

1. **依赖**：`server/requirements.txt` 与 `web/package-lock.json` 的已知漏洞（pip-audit / npm audit
   或等价离线核查）；记录版本、CVE、处置（升级/接受/无法修）。
2. **路径/文件**：
   - 插件入口/日志/状态路径（目录穿越、同前缀兄弟目录、符号链接）；
   - 证据包/截图/视频/导出（严格 data_dir 包含、文件名注入、zip 路径）；
   - folder 图片源路径校验。
3. **外联/SSRF**：MQTT host、GenAI URL（回环限制）、更新清单（https/不跟重定向）、Webhook 白名单、
   发现助手（仅 RFC1918）、告警渠道域名白名单。
4. **令牌/凭据**：admin/read/plugin token 不回显/不落日志/不进诊断包；MQTT_PASS、GENAI_API_KEY、
   SMTP_PASS、webhook secret 脱敏清单；PWA 缓存不缓存 API/带 token 响应。
5. **上传/导入**：配置导入、人脸图片、插件清单大小/字段限制；zip 炸弹/超大文件。
6. **插件执行**：默认禁用、权限说明、子进程环境清理、Popen 参数（无 shell）、日志不泄 token。
7. **CSP/安全头/PWA**：SW 缓存范围不越权；更新检查返回 URL 仅展示不自动打开。

## 交付

- `docs/75-security-audit-2026-11-05.md`：清单表（项/结论 PASS|WARN|FAIL/证据/修复/遗留）；
- 高危/中危修复 + 回归测试；无法修的写 BLOCKED/接受理由；
- 不写任何真实密钥；必要时用占位符。

## 验收

- `pytest tests/ -q` 全绿；`npm run build` 通过；
- 审计报告无 FAIL（或 FAIL 均有明确接受/阻塞说明）；
- 抽查：插件兄弟目录、folder 越界、MQTT 密码脱敏、诊断包不含新密钥。

## 约束

- 不引入安全扫描服务/云 API；不做破坏性渗透/DoS；
- 不改业务功能（除安全修复）；不发布。
