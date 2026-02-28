import 'dart:async';

import 'package:fl_chart/fl_chart.dart';
import 'package:flutter/material.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import '../services/app_channel_client.dart';
import '../services/app_state.dart';

class DashboardPage extends StatefulWidget {
  const DashboardPage({super.key});

  @override
  State<DashboardPage> createState() => _DashboardPageState();
}

class _DashboardPageState extends State<DashboardPage> {
  final AppChannelClient _client = const AppChannelClient();
  late final AppStateController _controller;

  Timer? _streamRetryTimer;
  WebSocketChannel? _eventChannel;
  StreamSubscription<Map<String, dynamic>>? _eventSub;

  @override
  void initState() {
    super.initState();
    _controller = AppStateController(client: _client);
    _controller.start();
    _connectRealtimeStream();
  }

  @override
  void dispose() {
    _streamRetryTimer?.cancel();
    _eventSub?.cancel();
    final channel = _eventChannel;
    _eventChannel = null;
    if (channel != null) {
      _client.closeEventChannel(channel);
    }
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, _) {
        final s = _controller.snapshot;
        return ListView(
          padding: const EdgeInsets.all(16),
          children: [
            _ProgressHeader(
              taskId: s.taskId,
              progress: s.progress,
              status: s.status,
              phase: s.phase,
              updatedAt: s.updatedAt,
              streamConnected: s.streamConnected,
              lastEventAt: s.lastEventAt,
            ),
            const SizedBox(height: 16),
            _SummaryCard(summary: s.summary, statusHint: s.statusHint),
            const SizedBox(height: 16),
            _CheckpointRow(progress: s.progress),
            const SizedBox(height: 20),
            const Text(
              '服务器状态',
              style: TextStyle(fontWeight: FontWeight.w700, fontSize: 18),
            ),
            const SizedBox(height: 10),
            _MetricChartCard(
              title: 'CPU',
              suffix: '%',
              color: const Color(0xFF0EA5E9),
              values: s.metrics.cpu,
            ),
            const SizedBox(height: 12),
            _MetricChartCard(
              title: 'RAM',
              suffix: '%',
              color: const Color(0xFF10B981),
              values: s.metrics.ram,
            ),
            const SizedBox(height: 12),
            _MetricChartCard(
              title: 'ROM',
              suffix: '%',
              color: const Color(0xFFF59E0B),
              values: s.metrics.rom,
            ),
            if (s.loading)
              const Padding(
                padding: EdgeInsets.only(top: 16),
                child: Center(child: CircularProgressIndicator()),
              ),
          ],
        );
      },
    );
  }

  Future<void> _connectRealtimeStream() async {
    _streamRetryTimer?.cancel();
    await _eventSub?.cancel();

    final oldChannel = _eventChannel;
    _eventChannel = null;
    if (oldChannel != null) {
      await _client.closeEventChannel(oldChannel);
    }

    try {
      final channel = await _client.connectEventChannel();
      _eventChannel = channel;

      _controller.setStreamConnected(true, hint: '实时流已连接（轮询兜底开启）');

      _eventSub = _client
          .eventStream(channel)
          .listen(
            (event) => _controller.applyStreamEvent(event),
            onError: (Object error, StackTrace stackTrace) {
              _scheduleStreamReconnect('实时流异常：$error');
            },
            onDone: () {
              _scheduleStreamReconnect('实时流已断开，准备重连…');
            },
            cancelOnError: false,
          );
    } catch (e) {
      _scheduleStreamReconnect('实时流连接失败：$e');
    }
  }

  void _scheduleStreamReconnect(String reason) {
    _controller.setStreamConnected(false, hint: reason);

    _streamRetryTimer?.cancel();
    _streamRetryTimer = Timer(const Duration(seconds: 5), () {
      if (!mounted) return;
      _connectRealtimeStream();
    });
  }
}

class _ProgressHeader extends StatelessWidget {
  const _ProgressHeader({
    required this.taskId,
    required this.progress,
    required this.status,
    required this.phase,
    required this.updatedAt,
    required this.streamConnected,
    required this.lastEventAt,
  });

  final String taskId;
  final double progress;
  final String status;
  final String phase;
  final DateTime updatedAt;
  final bool streamConnected;
  final DateTime? lastEventAt;

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const Expanded(
                  child: Text(
                    '任务总进度',
                    style: TextStyle(fontWeight: FontWeight.w700, fontSize: 16),
                  ),
                ),
                Text(
                  '${progress.toStringAsFixed(1)}%',
                  style: const TextStyle(
                    fontWeight: FontWeight.w700,
                    fontSize: 20,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 10),
            LinearProgressIndicator(value: progress / 100),
            const SizedBox(height: 10),
            Text('任务状态：$status（阶段：$phase）'),
            const SizedBox(height: 4),
            Text(
              _statusLabel(status),
              style: TextStyle(
                color: _statusColor(status),
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              taskId.isEmpty ? '任务 ID：未创建' : '任务 ID：$taskId',
              style: Theme.of(context).textTheme.bodySmall,
            ),
            const SizedBox(height: 6),
            Text(
              '最近更新：${updatedAt.hour.toString().padLeft(2, '0')}:${updatedAt.minute.toString().padLeft(2, '0')}:${updatedAt.second.toString().padLeft(2, '0')}',
              style: Theme.of(context).textTheme.bodySmall,
            ),
            const SizedBox(height: 4),
            Text(
              streamConnected ? '实时流：在线' : '实时流：离线（轮询兜底）',
              style: Theme.of(context).textTheme.bodySmall,
            ),
            const SizedBox(height: 4),
            Text(
              '事件心跳：${_lastEventLabel(lastEventAt)}',
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
        ),
      ),
    );
  }

  String _statusLabel(String value) {
    switch (value) {
      case 'queued':
        return '当前状态：排队中';
      case 'running':
        return '当前状态：处理中';
      case 'succeeded':
        return '当前状态：已完成';
      case 'failed':
        return '当前状态：失败（可重试）';
      default:
        return '当前状态：空闲';
    }
  }

  Color _statusColor(String value) {
    switch (value) {
      case 'queued':
      case 'running':
        return Colors.blueGrey;
      case 'succeeded':
        return Colors.green;
      case 'failed':
        return Colors.red;
      default:
        return Colors.grey;
    }
  }

  String _lastEventLabel(DateTime? value) {
    if (value == null) return '暂无';
    final delta = DateTime.now().difference(value).inSeconds;
    if (delta < 1) return '刚刚';
    return '$delta 秒前';
  }
}

class _SummaryCard extends StatelessWidget {
  const _SummaryCard({required this.summary, required this.statusHint});

  final String summary;
  final String statusHint;

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                const Icon(Icons.auto_awesome, size: 20),
                const SizedBox(width: 8),
                Expanded(child: Text(summary)),
              ],
            ),
            const SizedBox(height: 8),
            Text(
              '状态：$statusHint',
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
        ),
      ),
    );
  }
}

class _CheckpointRow extends StatelessWidget {
  const _CheckpointRow({required this.progress});

  final double progress;

  @override
  Widget build(BuildContext context) {
    final checkpoints = <int>[30, 50, 70, 99];

    return Wrap(
      spacing: 8,
      runSpacing: 8,
      children: checkpoints.map((point) {
        final reached = progress >= point;
        return Chip(
          avatar: Icon(
            reached ? Icons.check_circle : Icons.radio_button_unchecked,
            size: 18,
            color: reached ? Colors.green : Colors.grey,
          ),
          label: Text('$point%'),
        );
      }).toList(),
    );
  }
}

class _MetricChartCard extends StatelessWidget {
  const _MetricChartCard({
    required this.title,
    required this.suffix,
    required this.color,
    required this.values,
  });

  final String title;
  final String suffix;
  final Color color;
  final List<double> values;

  @override
  Widget build(BuildContext context) {
    final current = values.isEmpty ? 0 : values.last;

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Text(
                  '$title ${current.toStringAsFixed(1)}$suffix',
                  style: const TextStyle(fontWeight: FontWeight.w600),
                ),
                const Spacer(),
                Text(
                  '${values.length} points',
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              ],
            ),
            const SizedBox(height: 8),
            SizedBox(
              height: 120,
              child: LineChart(
                LineChartData(
                  minY: 0,
                  maxY: 100,
                  titlesData: const FlTitlesData(show: false),
                  gridData: const FlGridData(show: false),
                  borderData: FlBorderData(show: false),
                  lineBarsData: [
                    LineChartBarData(
                      spots: values
                          .asMap()
                          .entries
                          .map(
                            (entry) =>
                                FlSpot(entry.key.toDouble(), entry.value),
                          )
                          .toList(),
                      isCurved: true,
                      color: color,
                      barWidth: 2.5,
                      dotData: const FlDotData(show: false),
                      belowBarData: BarAreaData(
                        show: true,
                        color: color.withValues(alpha: 0.16),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
