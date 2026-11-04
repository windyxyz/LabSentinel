"""告警规则引擎：区域 × 时段 × 行为 × 级别 × 冷却期。"""
import time
from datetime import datetime

from .store import Store


class RuleEngine:
    PRUNE_EVERY_CALLS = 1000  # 每 N 次 match 清理一次过期冷却键（避免每帧全表扫描）

    def __init__(self, store: Store):
        self.store = store
        self._last_fire: dict[tuple, float] = {}  # (rule_id, track_id_or_0) -> monotonic ts
        self._prune_calls = 0
        self._last_prune = time.monotonic()

    def _in_time_window(self, rule, now: datetime) -> bool:
        hhmm = now.strftime("%H:%M")
        start, end = rule["time_start"], rule["time_end"]
        if start <= end:
            return start <= hhmm <= end
        return hhmm >= start or hhmm <= end  # 跨午夜

    def _matches_calendar(self, rule, now: datetime) -> bool:
        """星期/假期条件（任务112）：days_of_week（ISO 1=周一…7=周日）+ holiday_only。

        缺列（None）按全星期；**空串/非法串按不匹配**（坏数据 fail-closed，不产生事件不抛异常）；
        holiday_only=1 只在 is_holiday 为真的日期生效；=0 时假期不特殊（仍按时间/星期）。
        """
        raw = rule.get("days_of_week")
        if raw is None:
            raw = "1,2,3,4,5,6,7"  # 旧库未迁移的兜底（正常路径 v6 已带默认值）
        allowed = set()
        for part in str(raw).split(","):
            part = part.strip()
            if part.isdigit() and 1 <= int(part) <= 7:
                allowed.add(part)
        if not allowed or str(now.isoweekday()) not in allowed:
            return False
        if rule.get("holiday_only") and not self.store.is_holiday(now.date()):
            return False
        return True

    def prune(self, now: float | None = None, max_age_s: int | None = None) -> int:
        """回收过期冷却键（106）：超过 max(cooldown, 3600s) 的记录已不可能再抑制告警。

        返回删除条数。max_age_s 缺省按当前全部规则的最大冷却期兜底，避免把长冷却规则误清。
        """
        now = time.monotonic() if now is None else now
        if max_age_s is None:
            cooldowns = [int(r.get("cooldown_s") or 0) for r in self.store.rules()]
            max_age_s = max([3600, *cooldowns])
        cutoff = now - max(0, int(max_age_s))
        stale = [key for key, ts in self._last_fire.items() if ts < cutoff]
        for key in stale:
            self._last_fire.pop(key, None)
        self._last_prune = now
        return len(stale)

    def _maybe_prune(self, now: float) -> None:
        self._prune_calls += 1
        if self._prune_calls >= self.PRUNE_EVERY_CALLS:
            self._prune_calls = 0
            self.prune(now=now)

    def match(self, camera_id: int, zone_id: int | None, behavior: str,
              track_id: int = 0, level_hint: int | None = None):
        """返回命中的规则（含冷却判定）。zone_id=None 表示无区域规则（occupancy）。"""
        now_mono = time.monotonic()
        self._maybe_prune(now_mono)
        now = datetime.now()
        for rule in self.store.rules(camera_id):
            if not rule["enabled"] or rule["behavior"] != behavior:
                continue
            if rule["zone_id"] is not None and zone_id is not None and rule["zone_id"] != zone_id:
                continue
            if not self._matches_calendar(rule, now):  # 任务112：星期/假期条件
                continue
            if not self._in_time_window(rule, now):
                continue
            key = (rule["id"], track_id)
            if now_mono - self._last_fire.get(key, -1e9) < rule["cooldown_s"]:
                continue
            self._last_fire[key] = now_mono
            return rule
        return None

    def occupancy_rules(self, camera_id: int):
        return [r for r in self.store.rules(camera_id)
                if r["enabled"] and r["behavior"] == "occupancy_over"]
