// 服务端访问层：REST（http 包）+ WebSocket（内置 dart:io）。
// ApiClient 继承 ChangeNotifier，经 provider 注入全局（见 main.dart）：
// baseUrl 的加载/变更会 notifyListeners，依赖方（设置页、事件列表）自动重建。
import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

import '../models/event.dart';

/// 业务异常：把状态码 / 网络错误转成带中文说明的异常，页面捕获后可直接展示。
class ApiClientException implements Exception {
  ApiClientException(this.message);

  final String message;

  @override
  String toString() => message; // 直接当文案用，不再包一层 Exception
}

/// 服务端客户端：服务器地址可配置并持久化，事件列表 / 截图 / 连通性测试 / WS 实时监听。
class ApiClient extends ChangeNotifier {
  /// shared_preferences 的持久化键
  static const String _prefsKeyBaseUrl = 'server_base_url';

  /// HTTP 请求超时（WS 建连也复用）
  static const Duration _timeout = Duration(seconds: 8);

  /// WS 断线后的重连间隔
  static const Duration _wsReconnectDelay = Duration(seconds: 3);

  String _baseUrl = '';
  bool _ready = false;
  SharedPreferences? _prefs;

  /// 当前服务器地址（形如 http://192.168.1.10:8000）；空串表示未配置
  String get baseUrl => _baseUrl;

  /// 持久化的地址是否已从磁盘恢复（避免首帧按「未配置」渲染导致闪动）
  bool get ready => _ready;

  /// 是否已配置服务器地址
  bool get isConfigured => _baseUrl.isNotEmpty;

  // ================= 服务器地址 =================

  /// 启动时从本地持久化恢复地址（main.dart 在 create 后调用）
  Future<void> load() async {
    _prefs = _prefs ?? await SharedPreferences.getInstance();
    _baseUrl = _prefs!.getString(_prefsKeyBaseUrl) ?? '';
    _ready = true;
    notifyListeners();
  }

  /// 设置并持久化服务器地址；自动补 http:// 前缀、去掉尾部斜杠
  Future<void> setBaseUrl(String raw) async {
    var url = raw.trim();
    if (url.isNotEmpty && !url.startsWith(RegExp(r'^https?://'))) {
      url = 'http://$url'; // 局域网部署默认 http 明文
    }
    while (url.endsWith('/')) {
      url = url.substring(0, url.length - 1);
    }
    _baseUrl = url;
    final prefs = _prefs ?? await SharedPreferences.getInstance();
    await prefs.setString(_prefsKeyBaseUrl, url);
    notifyListeners();
  }

  // ================= REST =================

  /// 拉取事件列表（服务端按 id 倒序返回，最新在前）。
  /// [limit] 条数上限；[cameraId] / [level] 可选过滤，对齐 GET /api/events 参数。
  Future<List<Event>> fetchEvents({
    int limit = 50,
    int? cameraId,
    int? level,
  }) async {
    final query = <String>['limit=$limit'];
    if (cameraId != null) query.add('camera_id=$cameraId');
    if (level != null) query.add('level=$level');
    final data = await _getJson('/api/events?${query.join('&')}');
    if (data is! List) {
      throw ApiClientException('事件接口返回格式异常');
    }
    return data
        .map((e) => Event.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  /// 连通性测试：GET /api/system（版本 / 推理后端 / 摄像头状态 / 今日计数）
  Future<Map<String, dynamic>> fetchSystem() async {
    final data = await _getJson('/api/system');
    if (data is! Map<String, dynamic>) {
      throw ApiClientException('system 接口返回格式异常');
    }
    return data;
  }

  /// 事件截图地址：GET /api/events/{id}/snapshot（返回 image/jpeg）。
  /// 未配置地址时返回 null，由调用方展示占位。
  String? snapshotUrl(int eventId) =>
      isConfigured ? '$_baseUrl/api/events/$eventId/snapshot' : null;

  /// 统一 GET + JSON 解析；把各类网络错误翻译成中文提示
  Future<dynamic> _getJson(String path) async {
    if (!isConfigured) {
      throw ApiClientException('未配置服务器地址，请先到「设置」页填写');
    }
    final http.Response resp;
    try {
      resp = await http
          .get(
            Uri.parse('$_baseUrl$path'),
            headers: const {'Accept': 'application/json'},
          )
          .timeout(_timeout);
    } on TimeoutException {
      throw ApiClientException('请求超时：$_baseUrl$path');
    } on SocketException catch (e) {
      // 真机最常见的坑：填了 localhost/127.0.0.1（那是手机自己）
      throw ApiClientException(
          '网络错误：${e.message}（真机请填服务器的局域网 IP，不能是 localhost）');
    } catch (e) {
      throw ApiClientException('请求失败：$e');
    }
    if (resp.statusCode != 200) {
      throw ApiClientException('HTTP ${resp.statusCode}（$path）');
    }
    // http 包在响应头缺 charset 时按 latin1 解码 body，中文会乱码 ——
    // 必须用 bodyBytes 手动按 utf-8 解码
    return jsonDecode(utf8.decode(resp.bodyBytes));
  }

  // ================= WebSocket 实时事件 =================

  WebSocket? _ws; // 当前连接；null 表示未连上
  Timer? _reconnectTimer; // 断线重连定时器
  bool _wsWanted = false; // 用户期望的开关状态（区别于瞬时连接状态）
  bool _wsConnected = false; // 是否已连上（供 UI 展示）
  int _lastEventId = 0; // 查重基线：最近一次收到/见到的事件 id

  /// 新事件回调（已查重；由 AppState 接线到系统通知）
  void Function(Event event)? onEvent;

  /// WS 是否已连上
  bool get wsConnected => _wsConnected;

  /// WS 地址：http(s)://host:port 推导为 ws(s)://host:port + /ws/events
  String get _wsUrl =>
      '${_baseUrl.replaceFirst('http://', 'ws://').replaceFirst('https://', 'wss://')}/ws/events';

  /// 开启实时监听：先同步查重基线（把当前最新事件 id 记下，避免把历史事件当新事件通知），再建连
  Future<void> startWs() async {
    _wsWanted = true;
    await _syncBaseline();
    await _connectWs();
  }

  /// 关闭实时监听：取消重连计划并断开
  Future<void> stopWs() async {
    _wsWanted = false;
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    onEvent = null;
    final ws = _ws;
    _ws = null;
    if (ws != null) {
      try {
        await ws.close();
      } catch (_) {
        // 关闭失败无需处理（连接本身已不可用）
      }
    }
    if (_wsConnected) {
      _wsConnected = false;
      notifyListeners();
    }
  }

  /// 同步查重基线：取最新 1 条事件 id（只增不减，防止基线回退导致重复通知）
  Future<void> _syncBaseline() async {
    try {
      final latest = await fetchEvents(limit: 1);
      if (latest.isNotEmpty && latest.first.id > _lastEventId) {
        _lastEventId = latest.first.id;
      }
    } on ApiClientException {
      // 基线同步失败不致命：极端情况下开启后收到的首条事件会通知一次
    }
  }

  /// 建立 WS 连接；失败则进入自动重连循环
  Future<void> _connectWs() async {
    if (!_wsWanted || !isConfigured) return;
    try {
      final ws = await WebSocket.connect(_wsUrl).timeout(_timeout);
      _ws = ws;
      _wsConnected = true;
      notifyListeners();
      ws.listen(
        _onWsMessage,
        onDone: _onWsDisconnected,
        onError: (_) => _onWsDisconnected(),
        cancelOnError: true,
      );
    } catch (_) {
      // 服务端未开 / 地址错误 / 网络不通 → 延迟重连
      _scheduleReconnect();
    }
  }

  /// 服务端每广播一条事件就收到一段 JSON 文本（结构与 /api/events 行一致）
  void _onWsMessage(dynamic data) {
    try {
      final json = jsonDecode(data as String);
      if (json is! Map<String, dynamic>) return;
      final event = Event.fromJson(json);
      // 查重：只接受比基线更新的事件 id
      if (event.id <= _lastEventId) return;
      _lastEventId = event.id;
      onEvent?.call(event);
    } catch (_) {
      // 心跳或非 JSON 报文，忽略
    }
  }

  /// 连接断开（服务端重启 / 网络切换等）→ 置状态并自动重连
  void _onWsDisconnected() {
    _ws = null;
    if (!_wsWanted) return; // 用户已关闭开关，不再重连
    if (_wsConnected) {
      _wsConnected = false;
      notifyListeners();
    }
    _scheduleReconnect();
  }

  /// 自动重连（骨架阶段固定 3 秒间隔；后续可加退避策略）
  void _scheduleReconnect() {
    if (!_wsWanted) return;
    _reconnectTimer?.cancel();
    _reconnectTimer = Timer(_wsReconnectDelay, _connectWs);
  }

  @override
  void dispose() {
    _reconnectTimer?.cancel();
    _ws?.close();
    super.dispose();
  }
}
