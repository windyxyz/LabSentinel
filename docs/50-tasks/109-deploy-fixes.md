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

**flash 2026-11-04 完成（pytest 354 passed = 343+11 新增；`test_deploy_fixes.py` 11 用例全绿）**

### 1. Node 解压 rename 目标已存在 → `_promote_extracted_dir()` 降级链

- 新增 `scripts/deploy.py` `_promote_extracted_dir(extracted, dest)`：dest 已存在先
  `shutil.rmtree`（失败不中断，留给覆盖复制）→ `extracted.rename(dest)` → OSError 回退
  `copytree(extracted, dest, dirs_exist_ok=True)` 后删源。`ensure_node` 原裸 rename 调用点替换。
- 测试：fresh rename / 目标已有 stale 残留被整体替换（不是半新半旧）/ mock `Path.rename`
  必败回退 copytree（跨盘场景）/ mock rmtree 失败覆盖复制仍就位，4 用例。

### 2. ffmpeg-static 版本号 → `_version_key()` 数值排序

- 新增 `_version_key(name)`：`re.split(r"(\d+)")` 分段，数字段按数值、文本段按字典序
  （tagged 元组 `(0,int)/(1,str)` 避免 int/str 直接比较 TypeError）；不含数字的名字排最后。
  `sorted(versions)[-1]` → `max(versions, key=_version_key)`。
- 测试：b9.0<b10.0（字典序会选错的核心用例）、b6.0/b9.0/b10.0 全序、`latest` 排最后，2 用例。

### 3. ensure_env_file 只修模型行

- 模板分支不变（补 `env.parent.mkdir` 防御，测试沙箱无 server/ 目录也能生成）；
  更新分支重写为：定位 `LS_PPE_MODEL=` 行 **原位替换**（无则追加），其余行/注释逐字保留；
  写前 `copy2` 备份 `.env.bak-<ts>`（失败不阻塞）；"有效"判定逻辑不变（指向当前安装位置才跳过）。
- `_drop_env` 顺带清理 `.env.bak-*`（重部署/卸载不留备份残渣）。
- 测试：缺文件生成模板 / 有效配置逐字不动且无备份 / 模型失效仅改模型行+注释告警渠道保留+备份存在 /
  无模型行追加 / `_drop_env` 清理备份，5 用例。

### 验证

- `pytest tests -q`：**354 passed**（基线 343 + 新 11），2 warnings（既有 fastapi/pydantic 弃用提示）；
- 三处改动均为最小增量：部署流程顺序/其它逻辑未动，零新依赖，测试全临时目录+mock，
  真实 `server/.env` 未触碰（validity 判定经 `monkeypatch ROOT` 隔离）。

### 遗留

- 无。下一项 110（停止 PID 复用防护 + 冒烟清理）。
