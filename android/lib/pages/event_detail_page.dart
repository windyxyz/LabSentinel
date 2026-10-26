// 事件详情页：字段展示 + 截图查看（NetworkImage 拉取 /api/events/{id}/snapshot，
// 带加载与错误占位；点击可进全屏缩放查看）。
import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../api/client.dart';
import '../models/event.dart';
import '../theme.dart';

/// 事件详情
class EventDetailPage extends StatelessWidget {
  const EventDetailPage({super.key, required this.event});

  final Event event;

  @override
  Widget build(BuildContext context) {
    final api = context.watch<ApiClient>();
    final snapshotUrl = api.snapshotUrl(event.id);
    return Scaffold(
      appBar: AppBar(title: Text('事件 #${event.id}')),
      body: ListView(
        padding: const EdgeInsets.all(12),
        children: [
          // —— 截图区 ——
          _SnapshotCard(url: snapshotUrl, hasSnapshot: event.hasSnapshot),
          const SizedBox(height: 12),
          // —— 字段区 ——
          _FieldCard(event: event),
        ],
      ),
    );
  }
}

/// 截图卡片：有图加载 NetworkImage，无图/失败给占位
class _SnapshotCard extends StatelessWidget {
  const _SnapshotCard({required this.url, required this.hasSnapshot});

  final String? url;
  final bool hasSnapshot;

  @override
  Widget build(BuildContext context) {
    return _Panel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('现场截图', style: TextStyle(color: AppColors.muted, fontSize: 13)),
          const SizedBox(height: 8),
          if (!hasSnapshot || url == null)
            // 无截图占位
            Container(
              height: 160,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: AppColors.bg,
                borderRadius: BorderRadius.circular(6),
                border: Border.all(color: AppColors.border),
              ),
              child: const Text(
                '该事件无截图',
                style: TextStyle(color: AppColors.muted),
              ),
            )
          else
            // 有图：点击进全屏
            GestureDetector(
              onTap: () => Navigator.push(
                context,
                MaterialPageRoute(builder: (_) => _FullscreenSnapshotPage(url: url!)),
              ),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(6),
                child: SizedBox(
                  height: 220,
                  width: double.infinity,
                  // NetworkImage 自带 loading / error 回调，分别给进度与占位
                  child: Image.network(
                    url!,
                    fit: BoxFit.cover,
                    loadingBuilder: (context, child, progress) {
                      if (progress == null) return child; // 加载完成
                      return Container(
                        color: AppColors.bg,
                        alignment: Alignment.center,
                        child: const CircularProgressIndicator(strokeWidth: 2),
                      );
                    },
                    errorBuilder: (context, error, stackTrace) {
                      return Container(
                        color: AppColors.bg,
                        alignment: Alignment.center,
                        padding: const EdgeInsets.all(12),
                        child: const Text(
                          '截图加载失败\n（文件可能已被清理，或网络不可达）',
                          textAlign: TextAlign.center,
                          style: TextStyle(color: AppColors.muted, height: 1.6),
                        ),
                      );
                    },
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// 全屏截图查看：黑底 + 双指缩放/拖动，点击返回
class _FullscreenSnapshotPage extends StatelessWidget {
  const _FullscreenSnapshotPage({required this.url});

  final String url;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        title: const Text('截图'),
      ),
      body: GestureDetector(
        onTap: () => Navigator.pop(context),
        child: Center(
          child: InteractiveViewer(
            maxScale: 5, // 允许放大到 5 倍看细节
            child: Image.network(
              url,
              fit: BoxFit.contain,
              loadingBuilder: (context, child, progress) {
                if (progress == null) return child;
                return const Center(child: CircularProgressIndicator(strokeWidth: 2));
              },
              errorBuilder: (context, error, stackTrace) => const Text(
                '截图加载失败',
                style: TextStyle(color: AppColors.muted),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// 字段卡片：id / 类型 / 级别 / 时间 / 摄像头 / 轨迹 / 详情
class _FieldCard extends StatelessWidget {
  const _FieldCard({required this.event});

  final Event event;

  @override
  Widget build(BuildContext context) {
    return _Panel(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          // 类型 + 级别徽标
          Row(
            children: [
              Expanded(
                child: Text(
                  event.typeLabel,
                  style: const TextStyle(
                    color: AppColors.text,
                    fontSize: 17,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
              // 级别徽标：底色 15% 透明 + 级别色文字（对齐 Web 端 .lv 样式）
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
                decoration: BoxDecoration(
                  color: AppColors.levelColor(event.level).withAlpha(38), // 15% ≈ 38/255
                  borderRadius: BorderRadius.circular(10),
                ),
                child: Text(
                  event.levelLabel,
                  style: TextStyle(
                    color: AppColors.levelColor(event.level),
                    fontSize: 12,
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          _FieldRow(label: '时间', value: event.ts),
          _FieldRow(label: '摄像头', value: '#${event.cameraId}'),
          _FieldRow(label: '轨迹', value: event.trackId == null ? '-' : 'track ${event.trackId}'),
          _FieldRow(label: '事件 ID', value: '${event.id}'),
          const Divider(height: 20),
          // 详情整段展示
          const Text('详情', style: TextStyle(color: AppColors.muted, fontSize: 13)),
          const SizedBox(height: 4),
          SelectableText(
            event.detail.isEmpty ? '（无）' : event.detail,
            style: const TextStyle(color: AppColors.text, height: 1.5),
          ),
        ],
      ),
    );
  }
}

/// 单行字段：灰标签 + 值
class _FieldRow extends StatelessWidget {
  const _FieldRow({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: 64,
            child: Text(label, style: const TextStyle(color: AppColors.muted)),
          ),
          Expanded(
            child: SelectableText(value, style: const TextStyle(color: AppColors.text)),
          ),
        ],
      ),
    );
  }
}

/// 面板容器：对齐 Web 端 .panel（底色 --panel + 1px 描边 + 8px 圆角）
class _Panel extends StatelessWidget {
  const _Panel({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(
        color: AppColors.panel,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: AppColors.border),
      ),
      child: child,
    );
  }
}
