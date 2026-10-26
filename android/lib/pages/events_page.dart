// 事件列表页（首页）：下拉刷新 + 触底分页加载 + 左缘级别色条 + 点击进详情。
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../api/client.dart';
import '../models/event.dart';
import '../theme.dart';
import 'event_detail_page.dart';

/// 事件列表
class EventsPage extends StatefulWidget {
  const EventsPage({super.key});

  @override
  State<EventsPage> createState() => _EventsPageState();
}

class _EventsPageState extends State<EventsPage> {
  /// 每页条数（分页步长）
  static const int _pageSize = 20;

  List<Event> _events = [];
  bool _loading = false; // 首次加载中
  bool _loadingMore = false; // 加载更多中
  bool _hasMore = true; // 是否还有更旧的数据
  String? _error; // 首次加载失败原因

  @override
  void initState() {
    super.initState();
    // ApiClient 从磁盘恢复 baseUrl 是异步的：恢复完成后再触发首次加载
    WidgetsBinding.instance.addPostFrameCallback((_) => _initOnce());
  }

  Future<void> _initOnce() async {
    final api = context.read<ApiClient>();
    if (api.ready) {
      await refresh();
    } else {
      api.addListener(_onApiReady); // 等地址恢复完成
    }
  }

  void _onApiReady() {
    final api = context.read<ApiClient>();
    if (api.ready) {
      api.removeListener(_onApiReady);
      refresh();
    }
  }

  /// 下拉刷新：重置分页状态，重新拉最新一页
  Future<void> refresh() async {
    if (_loading) return;
    setState(() {
      _loading = true;
      _error = null;
      _hasMore = true;
    });
    try {
      final api = context.read<ApiClient>();
      if (!api.isConfigured) {
        throw ApiClientException('未配置服务器地址，请到「设置」页填写');
      }
      final events = await api.fetchEvents(limit: _pageSize);
      if (!mounted) return;
      setState(() {
        _events = events;
        _loading = false;
        // 返回不足一页说明没有更旧数据了
        _hasMore = events.length >= _pageSize;
      });
    } on ApiClientException catch (e) {
      if (!mounted) return;
      setState(() {
        _loading = false;
        _error = e.message;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _loading = false;
        _error = '加载失败：$e';
      });
    }
  }

  /// 加载更多：服务端 /api/events 暂无 offset 参数，
  /// 用「累计 limit」模拟分页（每页多取 _pageSize 条，按 id 倒序自然扩展）。
  Future<void> _loadMore() async {
    if (_loading || _loadingMore || !_hasMore) return;
    setState(() => _loadingMore = true);
    try {
      final api = context.read<ApiClient>();
      final nextLimit = _events.length + _pageSize;
      final events = await api.fetchEvents(limit: nextLimit);
      if (!mounted) return;
      setState(() {
        _events = events;
        _loadingMore = false;
        _hasMore = events.length >= nextLimit;
      });
    } catch (e) {
      if (!mounted) return;
      setState(() => _loadingMore = false);
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('加载更多失败：$e')));
    }
  }

  @override
  Widget build(BuildContext context) {
    final api = context.watch<ApiClient>();
    return Scaffold(
      appBar: AppBar(title: const Text('事件告警')),
      body: NotificationListener<ScrollNotification>(
        // 触底预加载：剩余可滚动距离不足 200px 时加载更多
        onNotification: (n) {
          if (n.metrics.extentAfter < 200) _loadMore();
          return false;
        },
        child: RefreshIndicator(
          onRefresh: refresh,
          child: _buildBody(api),
        ),
      ),
    );
  }

  /// 按状态渲染：未配置 → 首载中 → 出错 → 空 → 列表
  Widget _buildBody(ApiClient api) {
    if (api.ready && !api.isConfigured) {
      return _HintView(
        icon: Icons.dns_outlined,
        text: '尚未配置服务器地址\n请到下方「设置」页填写并保存',
      );
    }
    if (_loading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (_error != null) {
      return ListView(
        physics: const AlwaysScrollableScrollPhysics(),
        children: [
          const SizedBox(height: 80),
          _HintView(icon: Icons.cloud_off_outlined, text: _error!),
          const SizedBox(height: 16),
          Center(
            child: OutlinedButton.icon(
              onPressed: refresh,
              icon: const Icon(Icons.refresh),
              label: const Text('重试'),
            ),
          ),
        ],
      );
    }
    if (_events.isEmpty) {
      return ListView(
        physics: const AlwaysScrollableScrollPhysics(),
        children: const [
          SizedBox(height: 80),
          _HintView(icon: Icons.inbox_outlined, text: '暂无事件\n确认服务端摄像头已启动推理'),
        ],
      );
    }
    return ListView.builder(
      physics: const AlwaysScrollableScrollPhysics(),
      padding: const EdgeInsets.symmetric(vertical: 8),
      itemCount: _events.length + 1, // +1 为底部加载状态项
      itemBuilder: (context, i) {
        if (i == _events.length) {
          return _FooterView(
            loadingMore: _loadingMore,
            hasMore: _hasMore,
          );
        }
        final event = _events[i];
        return _EventTile(
          event: event,
          onTap: () => Navigator.push(
            context,
            MaterialPageRoute(
              builder: (_) => EventDetailPage(event: event),
            ),
          ),
        );
      },
    );
  }
}

/// 列表项：面板底色卡片 + 左缘级别色条（对齐 Web 端 --l1/--l2/--l3）
class _EventTile extends StatelessWidget {
  const _EventTile({required this.event, required this.onTap});

  final Event event;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 5, 12, 5),
      child: Material(
        color: AppColors.panel, // --panel
        borderRadius: BorderRadius.circular(8),
        child: InkWell(
          onTap: onTap,
          borderRadius: BorderRadius.circular(8),
          child: Container(
            padding: const EdgeInsets.fromLTRB(0, 10, 12, 10),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(8),
              border: Border.all(color: AppColors.border), // --border
            ),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                // 左缘级别色条
                Container(
                  width: 4,
                  margin: const EdgeInsets.symmetric(horizontal: 12),
                  decoration: BoxDecoration(
                    color: AppColors.levelColor(event.level),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          event.typeLabel,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                            color: AppColors.text,
                            fontWeight: FontWeight.w600,
                          ),
                        ),
                        const SizedBox(height: 4),
                        Text(
                          '${event.ts} · 摄像头 #${event.cameraId} · track ${event.trackId ?? "-"}',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style:
                              const TextStyle(color: AppColors.muted, fontSize: 12),
                        ),
                      ],
                    ),
                  ),
                ),
                const Icon(Icons.chevron_right, color: AppColors.muted),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 底部加载状态项：转圈（加载更多）/ 已全部加载
class _FooterView extends StatelessWidget {
  const _FooterView({required this.loadingMore, required this.hasMore});

  final bool loadingMore;
  final bool hasMore;

  @override
  Widget build(BuildContext context) {
    if (!hasMore) {
      return const Padding(
        padding: EdgeInsets.symmetric(vertical: 16),
        child: Center(
          child: Text('已加载全部', style: TextStyle(color: AppColors.muted)),
        ),
      );
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 16),
      child: Center(
        child: loadingMore
            ? const SizedBox(
                width: 20,
                height: 20,
                child: CircularProgressIndicator(strokeWidth: 2),
              )
            : const Text('上滑加载更多', style: TextStyle(color: AppColors.muted)),
      ),
    );
  }
}

/// 空态 / 错误提示
class _HintView extends StatelessWidget {
  const _HintView({required this.icon, required this.text});

  final IconData icon;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(icon, size: 48, color: AppColors.muted),
          const SizedBox(height: 12),
          Text(
            text,
            textAlign: TextAlign.center,
            style: const TextStyle(color: AppColors.muted, height: 1.6),
          ),
        ],
      ),
    );
  }
}
