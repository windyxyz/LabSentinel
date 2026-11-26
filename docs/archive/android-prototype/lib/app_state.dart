// 全局应用状态：WS 实时开关（持久化）+ 系统通知接线。
// 与 ApiClient 一起经 MultiProvider 注入（见 main.dart）。
import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'api/client.dart';
import 'models/event.dart';
import 'services/notification_service.dart';

/// 应用级设置与实时链路状态
class AppState extends ChangeNotifier {
  AppState(this._api);

  final ApiClient _api;
  final NotificationService _notifications = NotificationService();

  /// shared_preferences 的持久化键
  static const String _prefsKeyWsEnabled = 'ws_realtime_enabled';

  bool _wsEnabled = false; // 用户开关（持久化，启动自动恢复）
  bool _loaded = false;

  bool get wsEnabled => _wsEnabled;

  /// 设置是否已从磁盘恢复
  bool get loaded => _loaded;

  /// 启动时恢复设置；开关为开则自动建立 WS 监听（main.dart 在 create 后调用）
  Future<void> load() async {
    final prefs = await SharedPreferences.getInstance();
    _wsEnabled = prefs.getBool(_prefsKeyWsEnabled) ?? false;
    await _notifications.init();
    _loaded = true;
    if (_wsEnabled) {
      await _startListening();
    }
    notifyListeners();
  }

  /// 切换 WS 实时开关并持久化
  Future<void> setWsEnabled(bool value) async {
    if (_wsEnabled == value) return;
    _wsEnabled = value;
    final prefs = await SharedPreferences.getInstance();
    await prefs.setBool(_prefsKeyWsEnabled, value);
    if (value) {
      await _startListening();
    } else {
      await _api.stopWs();
    }
    notifyListeners();
  }

  /// 建立 WS 监听并把新事件回调接到系统通知
  Future<void> _startListening() async {
    if (!_api.isConfigured) return; // 未配置地址时静默等待，设置页会提示先配置
    _api.onEvent = _onNewEvent;
    await _api.startWs();
  }

  /// 收到新事件（ApiClient 已按最后事件 id 查重）→ 弹系统通知
  Future<void> _onNewEvent(Event event) async {
    try {
      await _notifications.showEvent(event);
    } catch (_) {
      // 通知失败（如权限被拒）不影响主流程
    }
  }

  @override
  void dispose() {
    _api.stopWs();
    super.dispose();
  }
}
