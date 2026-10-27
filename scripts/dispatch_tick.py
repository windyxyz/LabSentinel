"""巡检派发 tick（确定性，标准库）——供每 5 分钟的 cron 调用。

职责：
  1. 当前任务文档出现实质「## 执行报告」→ 标记完成，并派发队列下一个；
  2. 当前任务 45 分钟无进展 → 判定子代理额度耗尽：state=paused_quota，看板记一行，输出 AUTOSTOP；
  3. 无当前任务且队列非空 → 派发队首；
  4. 其它情况只更新 updated_ts 并在「最近巡检」留一行。

用法:
    python scripts/dispatch_tick.py
输出最后一行: STATUS=running|paused_quota / CURRENT=NN / DISPATCHED=NN / COMPLETED=NN / AUTOSTOP

设计说明：cron 只跑这一条命令，避免让模型自由发挥导致"只输出不落盘"。
看板写入为读-改-写（毫秒级窗口）；若与 flash 同时写看板，事后以文件内容为准。
"""
from __future__ import annotations

import json
import os
import re
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TASKS = ROOT / "docs" / "50-tasks"
STATE = TASKS / "派发状态.json"
BOARD = TASKS / "调度看板.md"
STALL_SECONDS = 45 * 60  # 用户 2026-10-28 调整：30 → 45 分钟
KEEP_RECENT = 8


def now_iso() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%S")


def hhmm() -> str:
    return time.strftime("%H:%M")


def mdhm() -> str:
    return time.strftime("%m-%d %H:%M")


def read_state() -> dict:
    return json.loads(STATE.read_text(encoding="utf-8"))


def write_state(state: dict) -> None:
    state["updated_ts"] = now_iso()
    state["updated_epoch"] = int(time.time())
    tmp = STATE.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(state, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(tmp, STATE)


def read_board() -> str:
    return BOARD.read_text(encoding="utf-8")


def write_board(text: str) -> None:
    tmp = BOARD.with_suffix(".md.tmp")
    tmp.write_text(text, encoding="utf-8")
    os.replace(tmp, BOARD)


def insert_after_header(text: str, header: str, line: str) -> str:
    """在指定二级标题后（跳过空行）插入一行。"""
    idx = text.find(header)
    if idx < 0:
        return text
    pos = text.find("\n", idx)
    if pos < 0:
        return text
    pos += 1
    if text[pos:pos + 1] == "\n":
        pos += 1
    return text[:pos] + line + "\n" + text[pos:]


def trim_recent(text: str, keep: int = KEEP_RECENT) -> str:
    """「最近巡检」只保留最新 keep 条 bullet。"""
    start = text.find("## 最近巡检")
    if start < 0:
        return text
    body_start = text.find("\n", start) + 1
    nxt = text.find("\n## ", body_start)
    body_end = len(text) if nxt < 0 else nxt
    body = text[body_start:body_end]
    lines = body.splitlines()
    bullets = [ln for ln in lines if ln.startswith("- ")]
    if len(bullets) <= keep:
        return text
    drop = set(bullets[keep:])
    kept = [ln for ln in lines if ln not in drop]
    return text[:body_start] + "\n".join(kept) + ("\n" if body.endswith("\n") else "") + text[body_end:]


def report_ok(doc_text: str) -> bool:
    """最后一个「## 执行报告」之后有实质内容（且不含占位符）。"""
    idx = doc_text.rfind("## 执行报告")
    if idx < 0:
        return False
    tail = doc_text[idx + len("## 执行报告"):].strip()
    return "（待填" not in tail and len(tail) > 100


def task_title(doc: Path) -> str:
    try:
        first = doc.read_text(encoding="utf-8").splitlines()[0].strip()
        return first.lstrip("# ").strip() or f"任务 {doc.name[:2]}"
    except (OSError, IndexError):
        return f"任务 {doc.name[:2]}"


def find_task_doc(num: str) -> Path | None:
    if not re.fullmatch(r"\d{2}", num):
        return None
    hits = sorted(TASKS.glob(f"{num}-*.md"))
    return hits[0] if hits else None


def main() -> int:
    state = read_state()
    board = read_board()

    if state.get("status") != "running":
        print(f"STATUS={state.get('status')}")
        if state.get("status") == "paused_quota":
            print("AUTOSTOP")
        return 0

    completed = None
    dispatched = None
    current = state.get("current_task")

    # 1) 当前任务：完成判定 / 停滞判定
    if current:
        doc = ROOT / state.get("current_task_doc", "")
        if not doc.exists():
            doc = find_task_doc(str(current))
        if doc and doc.exists():
            board_done = re.search(rf"任务\s*{re.escape(str(current))}\s*已交付待验收", board)
            if report_ok(doc.read_text(encoding="utf-8")) or board_done:
                completed = str(current)
                state["current_task"] = None
                state["last_progress_epoch"] = int(time.time())
                state["last_progress_iso"] = now_iso()
                state["last_progress_note"] = f"任务{current}完成"
                board = insert_after_header(
                    board, "## 进行中（flash 正在做）",
                    f"- 任务{current} 已交付待验收（{hhmm()}，详见任务文档执行报告）")
                board = insert_after_header(
                    board, "## 最近巡检",
                    f"- {mdhm()}（巡检）：**任务{current} 交付** → 待验收")
            else:
                age = int(time.time() - max(doc.stat().st_mtime, state.get("last_progress_epoch", 0)))
                if age >= STALL_SECONDS:
                    state["status"] = "paused_quota"
                    state["last_progress_note"] = f"任务{current} 45分钟无进展，疑似额度耗尽"
                    board = insert_after_header(
                        board, "## 进行中（flash 正在做）",
                        f"- ⏸ 暂停派发（疑似 flash 额度耗尽：任务{current} 45 分钟无进展）")
                    board = insert_after_header(
                        board, "## 最近巡检",
                        f"- {mdhm()}（巡检）：⏸ **暂停派发**——任务{current} 30 分钟无进展，"
                        f"疑似 flash 额度耗尽；已停止派发并删除定时")
                    state["status"] = "paused_quota"
                    board = trim_recent(board)
                    write_board(board)
                    write_state(state)
                    print(f"CURRENT={current}")
                    print("STATUS=paused_quota")
                    print("AUTOSTOP")
                    return 0
                # 无进展但仍在上限内：只记一行
                mins = max(1, age // 60)
                board = insert_after_header(
                    board, "## 最近巡检",
                    f"- {mdhm()}（巡检）：任务{current} 进行中（{mins} 分钟无新写入），队列 "
                    f"{state.get('queue', [])}")

    # 2) 派发队首
    if not state.get("current_task") and state.get("queue"):
        num = str(state["queue"][0])
        doc = find_task_doc(num)
        if doc is None:
            print(f"CURRENT=none\nSTATUS=running\nERROR=任务{num}文档不存在")
            return 1
        state["queue"] = state["queue"][1:]
        state["current_task"] = num
        state["current_task_doc"] = doc.relative_to(ROOT).as_posix()
        state["dispatched_at"] = now_iso()
        state["dispatch_epoch"] = int(time.time())
        state["task_doc_mtime_at_dispatch"] = int(doc.stat().st_mtime)
        state["last_progress_epoch"] = int(time.time())
        state["last_progress_iso"] = now_iso()
        state["last_progress_note"] = f"任务{num}已派发"
        dispatched = num
        board = insert_after_header(
            board, "## 进行中（flash 正在做）",
            f"- **{task_title(doc)}**（{hhmm()} 派发，待 flash 认领）："
            f"[{doc.name}](./{doc.name})")
        board = insert_after_header(
            board, "## 最近巡检",
            f"- {mdhm()}（巡检）：派发任务{num}（{doc.name}），队列剩余 {state['queue']}")

    board = trim_recent(board)
    write_board(board)
    write_state(state)

    print(f"STATUS={state.get('status')}")
    print(f"CURRENT={state.get('current_task') or 'none'}")
    if completed:
        print(f"COMPLETED={completed}")
    if dispatched:
        print(f"DISPATCHED={dispatched}")
    print(f"QUEUE={','.join(str(x) for x in state.get('queue', [])) or '-'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
