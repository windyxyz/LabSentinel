// 设置页：服务器地址（持久化）+ 连通性测试（GET /api/system）+ WS 实时通知开关。
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../api/client.dart';
import '../app_state.dart';
import '../theme.dart';

/// 设置
class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  late final ApiClient _api;
  late final TextEditingController _urlCtrl;
  bool _testing = false; // 测试连接进行中
  String? _testResult; // 测试成功信息
  String? _testError; // 测试失败信息

  @override
  void initState() {
    super.initState();
    _api = context.read<ApiClient>();
    _urlCtrl = TextEditingController(text: _api.baseUrl);
    // 持久化地址是异步恢复的：恢复完成后回填输入框
    _api.addListener(_onApiChanged);
  }

  void _onApiChanged() {
    if (_api.ready && mounted && _urlCtrl.text != _api.baseUrl) {
      _urlCtrl.text = _api.baseUrl;
    }
  }

  @override
  void dispose() {
    _api.removeListener(_onApiChanged);
    _urlCtrl.dispose();
    super.dispose();
  }

  /// 保存地址；若 WS 开关开着，用新地址重建实时监听
  Future<void> _save() async {
    await _api.setBaseUrl(_urlCtrl.text);
    final app = context.read<AppState>();
    if (app.wsEnabled) {
      await app.setWsEnabled(false); // 旧的 WS 连接已失效
      await app.setWsEnabled(true);
    }
    if (!mounted) return;
    ScaffoldMessenger.of(context)
        .showSnackBar(const SnackBar(content: Text('已保存服务器地址')));
  }

  /// 测试连接：先保存输入（所见即所存），再 GET /api/system
  Future<void> _test() async {
    setState(() {
      _testing = true;
      _testResult = null;
      _testError = null;
    });
    try {
      await _api.setBaseUrl(_urlCtrl.text);
      final sys = await _api.fetchSystem();
      final cameras = (sys['cameras'] as List?) ?? const [];
      final running = cameras.where((c) => c['status'] == 'running').length;
      if (!mounted) return;
      setState(() {
        _testing = false;
        _testResult = '连接成功 · 版本 ${sys["version"] ?? "?"} · '
            '推理后端 ${sys["backend"] ?? "?"} · 摄像头 $running/${cameras.length} 运行中';
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _testing = false;
        _testError = '$e';
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final app = context.watch<AppState>();
    return Scaffold(
      appBar: AppBar(title: const Text('设置')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          // ================= 服务器 =================
          const Text('服务器', style: TextStyle(color: AppColors.muted, fontSize: 13)),
          const SizedBox(height: 8),
          TextField(
            controller: _urlCtrl,
            keyboardType: TextInputType.url,
            style: const TextStyle(color: AppColors.text),
            decoration: const InputDecoration(
              hintText: '如 192.168.1.10:8000（自动补 http://）',
              labelText: '服务端地址',
              labelStyle: TextStyle(color: AppColors.muted),
            ),
            onSubmitted: (_) => _save(),
          ),
          const SizedBox(height: 4),
          const Text(
            '手机需与服务器同一局域网；真机上不能填 localhost / 127.0.0.1（那是手机自己）',
            style: TextStyle(color: AppColors.muted, fontSize: 12, height: 1.5),
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              // 保存按钮：实底强调色，对齐 Web 端 button 样式
              FilledButton(onPressed: _save, child: const Text('保存')),
              const SizedBox(width: 10),
              // 测试连接按钮：ghost 描边样式，对齐 Web 端 button.ghost
              OutlinedButton.icon(
                onPressed: _testing ? null : _test,
                icon: _testing
                    ? const SizedBox(
                        width: 14,
                        height: 14,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : const Icon(Icons.wifi_tethering, size: 18),
                label: const Text('测试连接'),
              ),
            ],
          ),
          // 测试结果 / 失败原因
          if (_testResult != null)
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: Text(_testResult!,
                  style: const TextStyle(color: AppColors.accent, height: 1.5)),
            ),
          if (_testError != null)
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: Text(_testError!,
                  style: const TextStyle(color: AppColors.l3, height: 1.5)),
            ),

          const SizedBox(height: 24),

          // ================= 实时通知 =================
          const Text('实时通知', style: TextStyle(color: AppColors.muted, fontSize: 13)),
          const SizedBox(height: 8),
          Container(
            decoration: BoxDecoration(
              color: AppColors.panel,
              borderRadius: BorderRadius.circular(8),
              border: Border.all(color: AppColors.border),
            ),
            child: SwitchListTile(
              value: app.wsEnabled,
              // 设置尚未恢复完时禁用；未配置地址时开了也无效，给提示并保持关闭
              onChanged: app.loaded
                  ? (v) async {
                      if (v && !_api.isConfigured) {
                        ScaffoldMessenger.of(context).showSnackBar(
                            const SnackBar(content: Text('请先填写并保存服务器地址')));
                        return;
                      }
                      await context.read<AppState>().setWsEnabled(v);
                    }
                  : null,
              title: const Text('WS 实时事件推送',
                  style: TextStyle(color: AppColors.text)),
              subtitle: Text(_wsSubtitle(app),
                  style: const TextStyle(color: AppColors.muted, fontSize: 12)),
            ),
          ),
          const SizedBox(height: 4),
          const Text(
            '开启后通过 WebSocket 连接 /ws/events，收到新事件弹系统通知'
            '（Android 13+ 首次开启会申请通知权限）',
            style: TextStyle(color: AppColors.muted, fontSize: 12, height: 1.5),
          ),
        ],
      ),
    );
  }

  /// 开关副标题：未配置 / 连接状态
  String _wsSubtitle(AppState app) {
    if (!app.loaded) return '正在恢复设置…';
    if (!_api.isConfigured) return '请先填写并保存服务器地址';
    if (app.wsEnabled) {
      return _api.wsConnected ? '已连接 /ws/events' : '连接中…（断线自动重连）';
    }
    return '关闭中';
  }
}
