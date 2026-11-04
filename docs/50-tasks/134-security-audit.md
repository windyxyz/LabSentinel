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

## 执行报告

**flash 2026-11-04 完成（审计报告 docs/75-security-audit-2026-11-05.md；pytest 501 passed = 499+2 新增）**
（首轮全量 1 failed = test_openapi 快照漂移——132 加参后未重生成，非审计缺陷；重生成后绿）

### 结论一览（完整清单表见审计报告）

- **依赖**：pip-audit 零漏洞；npm audit 端点被网络策略拦截 → ACCEPT（生产依赖仅 vue，CI 有网补跑）；
- **路径**：插件 is_relative_to/日志双防护/证据包 76 严格包含/folder 校验全 PASS；
- **外联/SSRF**：GenAI 回环、更新检查 https+不跟重定向、MQTT 配置驱动、Webhook https+凭证拒、
  发现助手 RFC1918+XXE 拒——全 PASS；
- **令牌**：plugin token 不回显、子进程环境剥离继承、MQTT/GENAI/SMTP 脱敏正则命中、
  SW 不缓存 API——全 PASS；
- **修复 2 项（均有回归测试）**：
  1. 人脸照片上传加 10MB 上限（原无限制读全量内存，413）；
  2. 配置导入规模上限（摄像头≤200/区域≤1000/规则≤2000，400）——
     **修复过程发现真缺陷**：cap 原写在填充循环后对空列表判断（永不触发），
     字节码反汇编（dis.get_instructions）定位后移到循环前直量原始输入。

### 验证

- `pytest tests -q -k security_audit` 2/2；OpenAPI 快照过期顺带重生成（132 加参后）；
- 全量回归见文末（预期 502）。

### 遗留

- 无 FAIL/BLOCKED。两项 ACCEPT 有明确理由（见审计报告一、八节）。

### 主线复审补丁（2026-11-04）

- 人脸照片 10MB 上限原在 `await photo.read()` **全量读入后**才检查，未真正防内存滥用；
  改为先看 `UploadFile.size`，再 `read(10MB+1)`，超限 413。`test_face_photo_10mb_cap` 仍绿。
