# CLAUDE.md — 多工具桥接规则

本仓库采用**接力开发模式**，请优先按以下顺序恢复上下文并严格遵守规则：

1. 全文阅读 [`AGENTS.md`](./AGENTS.md)
2. 查阅 [`task_registry.md`](docs/maintenance/task_registry.md) 锁定当前活跃 `Task-ID`
3. 读取 [`progress.md`](docs/maintenance/progress.md) 顶部 `Latest Handoff Snapshot`
4. 按需查阅 [`task_plan.md`](docs/maintenance/task_plan.md)、[`findings.md`](docs/maintenance/findings.md) 与 [`修改记录_会话备忘.md`](docs/maintenance/修改记录_会话备忘.md) 最近 1~3 条记录

核心原则：先锁定单一 `Task-ID` → 严格执行启动检查 → 未验证不宣称完成 → 结束前落盘三件套（`progress.md` / `修改记录_会话备忘.md` / `task_registry.md`）。
