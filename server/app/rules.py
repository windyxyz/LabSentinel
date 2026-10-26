"""告警规则引擎：区域 × 时段 × 行为 × 级别 × 冷却期。"""
from datetime import datetime

from .store import Store


class RuleEngine:
    def __init__(self, store: Store):
        self.store = store
        self._last_fire: dict[tuple, float] = {}  # (rule_id, track_id_or_0) -> monotonic ts

    def _in_time_window(self, rule, now: datetime) -> bool:
        hhmm = now.strftime("%H:%M")
        start, end = rule["time_start"], rule["time_end"]
        if start <= end:
            return start <= hhmm <= end
        return hhmm >= start or hhmm <= end  # 跨午夜

    def match(self, camera_id: int, zone_id: int | None, behavior: str,
              track_id: int = 0, level_hint: int | None = None):
        """返回命中的规则（含冷却判定）。zone_id=None 表示无区域规则（occupancy）。"""
        import time
        now = datetime.now()
        for rule in self.store.rules(camera_id):
            if not rule["enabled"] or rule["behavior"] != behavior:
                continue
            if rule["zone_id"] is not None and zone_id is not None and rule["zone_id"] != zone_id:
                continue
            if not self._in_time_window(rule, now):
                continue
            key = (rule["id"], track_id)
            if time.monotonic() - self._last_fire.get(key, -1e9) < rule["cooldown_s"]:
                continue
            self._last_fire[key] = time.monotonic()
            return rule
        return None

    def occupancy_rules(self, camera_id: int):
        return [r for r in self.store.rules(camera_id)
                if r["enabled"] and r["behavior"] == "occupancy_over"]
