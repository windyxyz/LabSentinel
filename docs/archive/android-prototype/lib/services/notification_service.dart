// 系统通知封装：基于 flutter_local_notifications，仅使用 Android 平台能力。
// 由 AppState 接线：WS 实时开关开启后，每收到一条新事件弹一条系统通知。
import 'package:flutter_local_notifications/flutter_local_notifications.dart';

import '../models/event.dart';
import '../theme.dart';

/// 事件告警通知服务（初始化幂等，可重复调用）
class NotificationService {
  final FlutterLocalNotificationsPlugin _plugin =
      FlutterLocalNotificationsPlugin();

  bool _initialized = false;

  /// 初始化插件并申请 Android 13+ 的通知运行时权限
  Future<void> init() async {
    if (_initialized) return;
    const androidInit = AndroidInitializationSettings('@mipmap/ic_launcher');
    await _plugin.initialize(const InitializationSettings(android: androidInit));
    // Android 13+ 需运行时申请 POST_NOTIFICATIONS；低版本系统调用无副作用
    final android = _plugin.resolvePlatformSpecificImplementation<
        AndroidFlutterLocalNotificationsPlugin>();
    await android?.requestNotificationsPermission();
    _initialized = true;
  }

  /// 弹出新事件通知：标题 = 级别，正文 = 类型 + 摄像头 + 详情
  Future<void> showEvent(Event event) async {
    await init();
    final details = AndroidNotificationDetails(
      'labsentinel_events', // 渠道 id
      'LabSentinel 事件告警', // 渠道名（系统设置中可见）
      channelDescription: 'WS 实时监听到的新事件通知',
      importance: Importance.high,
      priority: Priority.high,
      // 通知强调色对齐 Web 端 --accent
      color: AppColors.accent,
      colorized: true,
      onlyAlertOnce: true,
    );
    final body = '${event.typeLabel} · 摄像头 #${event.cameraId}\n${event.detail}';
    // 用事件 id 作通知 id：同一事件不会互相覆盖，新事件自然排在前面
    await _plugin.show(
      event.id,
      event.levelLabel,
      body,
      NotificationDetails(android: details),
    );
  }
}
