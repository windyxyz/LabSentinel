# 任务 76 · 路径/URI 安全与边界加固（P0/P1）

> 背景：主线复核发现两处“看起来能用、边界会出事”的实现：
> 1) `retention._safe_within` 用子串判断（`str(parent) in str(path)`），`C:\data-evil` 会被误判在 `C:\data` 内；
> 2) 多处只读 SQLite 用 `f"file:{path}?mode=ro"`，路径含空格/中文/`#`/`?` 时 URI 解析可能失败。
> 本任务统一加固并补测试，不改业务语义。

## 1. `_safe_within` 改为真实路径包含判断

- `server/app/retention.py`：
  ```python
  from pathlib import Path
  def _safe_within(path: str, parent) -> bool:
      try:
          p = Path(path).resolve()
          base = Path(parent).resolve()
          return p == base or base in p.parents
      except (OSError, ValueError):
          return False
  ```
  （Python 3.12 可用 `p.is_relative_to(base)`；项目兼容 3.10+ 时用 parents 写法。）
- `preview_cleanup` 与 `run_cleanup` 共用该函数；删除逻辑不变。
- 测试 `test_retention_preview.py`/`test_retention.py` 增加：
  - 正常子路径 → True；`parent` 同目录文件 → True；
  - 兄弟前缀 `data-evil/x`（与 data 同级）→ False，且预演不计字节、实删不删该文件；
  - `..\..\windows\x`、绝对路径越界 → False；
  - 不存在文件 → False 不崩。

## 2. SQLite 只读 URI 统一 helper

- 在 `server/app/store.py` 增加：
  ```python
  def ro_uri(path) -> str:
      from pathlib import Path
      return Path(path).resolve().as_uri() + "?mode=ro"
  ```
  `Path.as_uri()` 会正确百分号编码空格/中文/特殊字符（`file:///C:/a%20b/labsentinel.db`）。
- `store.connect(readonly=True)`、`api.health_ready` 的只读连接、`scripts/backup_data._snapshot_db`、
  `scripts/collect_diagnostics` 打开 db、`scripts/doctor` 的 integrity 检查，全部改用该 helper
  （脚本内无法 import app 时复制同一 5 行函数并注释“与 store.ro_uri 保持一致”）。
- 测试：临时目录名含 **空格 + 中文**（如 `数据 目录`），创建 db → 只读连接/备份/诊断/doctor 均成功；
  路径含 `#` 也测一次（URI 中 # 必须编码）。

## 3. 请求 ID 兜底响应头（与 75.6 合并；若 75 已做则跳过）

- `main.py` middleware catch-all 的 500 JSONResponse 带 `X-Request-ID`；测试覆盖。

## 4. 其他边界（顺手，不改行为）

- `retention._disk_free_gb` 异常返回 `inf` 会让“永远不会紧急清理”——改为返回 `0.0` 更安全？
  **不要直接改**：先写测试说明现状，若确认语义是“未知就当充足”可保留；在报告说明判断。
- `collect_diagnostics.sanitize_source` 对 file 路径返回 `***`；保持。
- `backup_data` 的 `_safe_entry` 已拒绝 `..`/绝对路径/盘符；补测 `..\\` 与 `C:foo`（当前是否覆盖）。

## 约束

- 不改表结构/清理阈值/备份格式；不引入依赖；
- 所有测试用临时目录；不碰真实 `server/data`；
- 不修改 `web/src`。

## 验收标准

- `pytest tests/ -q` 全绿（基线 165 + 新增）；
- 临时目录名含空格/中文/# 时：`store.connect(readonly=True)`、`backup_data backup/verify`、
  `collect_diagnostics`、`doctor` 全部通过；
- 兄弟前缀/穿越路径不删文件、不计入预演字节；
- 报告附测试用例清单与结果。

## 执行报告

（待填：改动文件 / 边界测试证据 / pytest / 遗留）
