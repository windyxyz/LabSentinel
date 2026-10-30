# 任务 109 · deploy.py 三处新机坑（P1）

> 来源：flash 审阅建议 #4；新机部署脚本的真实风险，用户已经反馈过类似问题。

## 1. Node 解压 rename 目标已存在即失败

- `scripts/deploy.py` 下载 Node 后 `extracted.rename(NODE_DIR)`；若 `tools/node` 已存在（上次半失败/
  手动放过），rename 在 Windows 上抛 `FileExistsError`，整个源被判失败，最后“Web 面板不可用”。
- 修复：rename 前若目标存在 → `shutil.rmtree`（失败则改用“复制后删源”）；rename 失败时回退
  `copytree(extracted, NODE_DIR, dirs_exist_ok=True)`；成功后清理 extracted/zip。
- 测试：用临时目录模拟“目标已存在”与“跨盘 rename 失败”（mock rename 抛 OSError）→ 最终 node.exe 就位。

## 2. ffmpeg-static 版本号字符串排序

- `sorted(versions)[-1]` 会把 `b9.0` 排在 `b10.0` 后（字典序），选到旧版。
- 修复：解析 `b(\d+)` / 数字段排序；无版本号的放最后；加纯函数 `_version_key(name)` 测试
  （b9.0/b10.0/b6.0 → b10.0 最大）。

## 3. ensure_env_file 重写会抹掉用户告警配置

- 模型路径失效时整文件重写，用户手加的 `LS_WECOM_WEBHOOK=...` 等全部丢失。
- 修复：只替换/追加 `LS_PPE_MODEL=` 一行；保留其它行与注释；写前备份 `.env.bak-<ts>`（可选）；
  若文件不存在仍生成模板。
- 测试：现有 `.env` 含自定义渠道 + 旧模型路径 → 只改模型行，其它行逐字保留；无模型行 → 追加。

## 约束

- 不改部署流程顺序/其它逻辑；不引入依赖；测试用临时目录与 mock；
- 不覆盖真实 `server/.env`（测试临时）；用户数据优先。

## 验收

- `pytest tests/ -q` 全绿（新增 `test_deploy_fixes.py`）；
- `deploy.py --help`/相关纯函数测试通过；报告附三段代码对照。

## 执行报告

（待填：改动 / 测试 / 遗留）
