"""T-321（L0）黄金语料事件 diff：比较两次 golden_run 输出的一致率。

匹配口径（76 文档 §11 T-321）：事件按 (camera, type, zone, 帧±1) 对齐——
本工具以 (camera, type, track_id) 分组、事件 ts（视频时间）±tol 秒贪心匹配；
zone 信息含于 detail（区域名），detail 不一致即视为不匹配（L0 要求逐字一致）。

退出码：0=一致率 100%；1=存在差异；2=参数/文件错误。
用法:
    python tools/golden_diff.py a.jsonl b.jsonl [--tol-s 0.21]
"""
import argparse
import json
import sys


def _load(p: str) -> list[dict]:
    rows = []
    with open(p, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def _ts_s(ts: str) -> float:
    # "2026-01-01T00:00:03" → 秒（相对固定基准日，仅用于同语料内的对齐）
    from datetime import datetime
    d = datetime.fromisoformat(ts)
    base = d.replace(hour=0, minute=0, second=0, microsecond=0)
    return (d - base).total_seconds()


def main() -> int:
    ap = argparse.ArgumentParser(description="黄金语料事件 diff（L0/L1 一致率）")
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--tol-s", type=float, default=0.21,
                    help="事件时间匹配容差秒（默认 1 个分析帧 @5fps=0.2s，留 10ms 余量）")
    a = ap.parse_args()

    try:
        rows_a = _load(a.a)
        rows_b = _load(a.b)
    except OSError as e:
        print(f"[diff] 文件读取失败: {e}")
        return 2

    def _match(x: list[dict], y: list[dict]) -> tuple[int, list[dict], list[dict]]:
        pool = list(y)
        matched = 0
        unmatched = []
        for r in x:
            key = (r["camera"], r["type"], r["track_id"])
            t = _ts_s(r["ts"])
            best_i, best_dt = None, None
            for i, q in enumerate(pool):
                if (q["camera"], q["type"], q["track_id"]) != key:
                    continue
                if q["detail"] != r["detail"]:
                    continue
                dt = abs(_ts_s(q["ts"]) - t)
                if dt <= a.tol_s and (best_dt is None or dt < best_dt):
                    best_i, best_dt = i, dt
            if best_i is None:
                unmatched.append(r)
            else:
                matched += 1
                pool.pop(best_i)
        return matched, unmatched, pool

    matched, unmatched_a, unmatched_b = _match(rows_a, rows_b)
    total = max(len(rows_a), len(rows_b))
    rate = matched / total if total else 1.0
    print(f"[diff] A={len(rows_a)} B={len(rows_b)} matched={matched} "
          f"unmatched_a={len(unmatched_a)} unmatched_b={len(unmatched_b)} "
          f"一致率={rate:.4f} (tol={a.tol_s}s)")
    for r in unmatched_a[:10]:
        print(f"  仅A: {r['type']} t={r['track_id']} ts={r['ts']} {r['detail']}")
    for r in unmatched_b[:10]:
        print(f"  仅B: {r['type']} t={r['track_id']} ts={r['ts']} {r['detail']}")
    return 0 if (not unmatched_a and not unmatched_b) else 1


if __name__ == "__main__":
    sys.exit(main())
