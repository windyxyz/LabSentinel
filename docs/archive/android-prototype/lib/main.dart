// LabSentinel 安卓客户端入口。
// 结构：MultiProvider 注入（ApiClient 网络层 + AppState 设置/实时链路）
//       → 深色主题（配色对齐 web/src/style.css）
//       → 底部导航 3 页：事件列表（首页）/ 实时画面（占位）/ 设置。
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import 'api/client.dart';
import 'app_state.dart';
import 'pages/events_page.dart';
import 'pages/settings_page.dart';
import 'theme.dart';

void main() {
  // shared_preferences / 通知插件的初始化需要绑定先就绪
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const LabSentinelApp());
}

class LabSentinelApp extends StatelessWidget {
  const LabSentinelApp({super.key});

  @override
  Widget build(BuildContext context) {
    // provider 做依赖注入：ApiClient（网络层）与 AppState（设置/WS 链路）
    return MultiProvider(
      providers: [
        // 网络层：create 后立即从磁盘恢复持久化的服务器地址
        ChangeNotifierProvider(create: (_) => ApiClient()..load()),
        // AppState 依赖 ApiClient —— providers 按顺序创建，此处可安全 read 到上一个
        ChangeNotifierProvider(
          create: (ctx) => AppState(ctx.read<ApiClient>())..load(),
        ),
      ],
      child: MaterialApp(
        title: 'LabSentinel',
        debugShowCheckedModeBanner: false,
        theme: buildAppTheme(),
        home: const HomeShell(),
      ),
    );
  }
}

/// 底部导航外壳：3 个页签，IndexedStack 切换时保留各页状态（列表滚动位置等）
class HomeShell extends StatefulWidget {
  const HomeShell({super.key});

  @override
  State<HomeShell> createState() => _HomeShellState();
}

class _HomeShellState extends State<HomeShell> {
  int _tab = 0; // 当前页签，0 = 事件列表（首页）

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: IndexedStack(
        index: _tab,
        children: const [
          EventsPage(),
          _LivePage(),
          SettingsPage(),
        ],
      ),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _tab,
        onDestinationSelected: (i) => setState(() => _tab = i),
        destinations: const [
          NavigationDestination(
            icon: Icon(Icons.notifications_outlined),
            selectedIcon: Icon(Icons.notifications),
            label: '事件',
          ),
          NavigationDestination(
            icon: Icon(Icons.videocam_outlined),
            selectedIcon: Icon(Icons.videocam),
            label: '实时',
          ),
          NavigationDestination(
            icon: Icon(Icons.settings_outlined),
            selectedIcon: Icon(Icons.settings),
            label: '设置',
          ),
        ],
      ),
    );
  }
}

/// 实时画面占位页 —— 等任务 01（go2rtc / WebRTC）就绪后接入
class _LivePage extends StatelessWidget {
  const _LivePage();

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          const Icon(Icons.videocam_off_outlined, size: 48, color: AppColors.muted),
          const SizedBox(height: 12),
          Text('实时画面占位', style: Theme.of(context).textTheme.titleMedium),
          const SizedBox(height: 4),
          const Text('后续接入任务 01 的 WebRTC 实时流',
              style: TextStyle(color: AppColors.muted)),
        ],
      ),
    );
  }
}
