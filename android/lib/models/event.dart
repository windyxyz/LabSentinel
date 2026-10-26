// 事件模型：与服务端 events 表字段一一对应（server/app/store.py）。
// REST（GET /api/events）与 WS（/ws/events 推送行）结构一致：
// id / ts / camera_id / type / level / track_id / detail / snapshot，
// 服务端会额外附加 has_snapshot 布尔字段（server/app/api.py 的 _event_row）。

/// 事件（告警）记录
class Event {
  const Event({
    required this.id,
    required this.ts,
    required this.cameraId,
    required this.type,
    required this.level,
    required this.detail,
    required this.hasSnapshot,
    this.trackId,
  });

  final int id; // 事件自增 id（查重 / 截图路由都用它）
  final String ts; // ISO 时间字符串，如 2026-10-25T12:00:00
  final int cameraId; // 来源摄像头 id
  final String type; // 行为类型原始值，如 enter_no_ppe
  final int level; // 告警级别：1 提示 / 2 警告 / 3 严重
  final int? trackId; // ByteTrack 轨迹 id（可能缺失）
  final String detail; // 详情文本
  final bool hasSnapshot; // 是否有截图（截图统一走 /api/events/{id}/snapshot）

  /// 从服务端 JSON 构造；字段全部做了空安全兜底，坏数据不至于崩 UI
  factory Event.fromJson(Map<String, dynamic> json) {
    final snapshotRaw = json['snapshot'];
    return Event(
      id: (json['id'] as num?)?.toInt() ?? 0,
      ts: json['ts'] as String? ?? '',
      cameraId: (json['camera_id'] as num?)?.toInt() ?? 0,
      type: json['type'] as String? ?? '',
      level: (json['level'] as num?)?.toInt() ?? 1,
      trackId: (json['track_id'] as num?)?.toInt(),
      detail: json['detail'] as String? ?? '',
      // REST 返回 has_snapshot；WS 原始行用 snapshot 路径非空判断，两者都兼容
      hasSnapshot: (json['has_snapshot'] as bool?) ??
          (snapshotRaw is String && snapshotRaw.isNotEmpty),
    );
  }

  /// 事件类型中文文案 —— 与 server/app/constants.py 的 TYPE_TEXT
  /// 及 web/src/api.js 的 TYPE_TEXT 保持同步（三份独立维护，改动要一起改）。
  static const Map<String, String> typeText = {
    'enter': '进入安全区',
    'enter_no_ppe': '进入安全区未穿实验服',
    'exit': '离开安全区',
    'occupancy_over': '在场人数超限',
  };

  /// 类型中文文案；未知类型直接展示原始值（以服务端为准）
  String get typeLabel => typeText[type] ?? type;

  /// 级别文案（L1 提示 / L2 警告 / L3 严重，与 constants.py 注释一致）
  String get levelLabel => switch (level) {
        1 => 'L1 提示',
        2 => 'L2 警告',
        3 => 'L3 严重',
        _ => 'L$level',
      };
}
