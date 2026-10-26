"""全局常量：事件类型中文映射、级别图标等。

注意：TYPE_TEXT 与前端 web/src/api.js 的同名导出**保持同步**（两份独立维护，
改动时两边一起改，前端另有 stranger 等新增类型以服务端为准）。
"""

# 事件类型 → 中文（前端 web/src/api.js TYPE_TEXT 同步维护）
TYPE_TEXT = {
    "enter": "进入安全区",
    "enter_no_ppe": "进入安全区未穿实验服",
    "exit": "离开安全区",
    "occupancy_over": "在场人数超限",
    "stranger": "陌生人进入",  # 人脸模块（默认关闭）产生
}

# 告警级别图标（L1 提示 / L2 警告 / L3 严重）
LEVEL_ICON = {1: "ℹ️", 2: "⚠️", 3: "🔴"}
