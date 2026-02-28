import 'dart:async';

import 'package:flutter/foundation.dart';

import 'app_channel_client.dart';

class AppMetrics {
  const AppMetrics({
    this.cpu = const <double>[],
    this.ram = const <double>[],
    this.rom = const <double>[],
  });

  final List<double> cpu;
  final List<double> ram;
  final List<double> rom;

  AppMetrics copyWith({
    List<double>? cpu,
    List<double>? ram,
    List<double>? rom,
  }) {
    return AppMetrics(
      cpu: cpu ?? this.cpu,
      ram: ram ?? this.ram,
      rom: rom ?? this.rom,
    );
  }
}

class AppSnapshot {
  const AppSnapshot({
    required this.taskId,
    required this.progress,
    required this.status,
    required this.phase,
    required this.summary,
    required this.metrics,
    required this.updatedAt,
    required this.streamConnected,
    required this.lastEventAt,
    required this.loading,
    required this.statusHint,
    this.error,
  });

  final String taskId;
  final double progress;
  final String status;
  final String phase;
  final String summary;
  final AppMetrics metrics;
  final DateTime updatedAt;
  final bool streamConnected;
  final DateTime? lastEventAt;
  final bool loading;
  final String statusHint;
  final Object? error;

  static AppSnapshot initial() {
    return AppSnapshot(
      taskId: '',
      progress: 0,
      status: 'idle',
      phase: 'idle',
      summary: '暂无任务，先去聊天页发送任务。',
      metrics: const AppMetrics(),
      updatedAt: DateTime.now(),
      streamConnected: false,
      lastEventAt: null,
      loading: true,
      statusHint: '正在拉取数据…',
      error: null,
    );
  }

  AppSnapshot copyWith({
    String? taskId,
    double? progress,
    String? status,
    String? phase,
    String? summary,
    AppMetrics? metrics,
    DateTime? updatedAt,
    bool? streamConnected,
    DateTime? lastEventAt,
    bool? loading,
    String? statusHint,
    Object? error,
    bool clearError = false,
  }) {
    return AppSnapshot(
      taskId: taskId ?? this.taskId,
      progress: progress ?? this.progress,
      status: status ?? this.status,
      phase: phase ?? this.phase,
      summary: summary ?? this.summary,
      metrics: metrics ?? this.metrics,
      updatedAt: updatedAt ?? this.updatedAt,
      streamConnected: streamConnected ?? this.streamConnected,
      lastEventAt: lastEventAt ?? this.lastEventAt,
      loading: loading ?? this.loading,
      statusHint: statusHint ?? this.statusHint,
      error: clearError ? null : (error ?? this.error),
    );
  }
}

class AppStateController extends ChangeNotifier {
  AppStateController({AppChannelClient? client})
    : _client = client ?? const AppChannelClient();

  final AppChannelClient _client;

  AppSnapshot _snapshot = AppSnapshot.initial();
  AppSnapshot get snapshot => _snapshot;

  Timer? _pollTimer;
  bool _started = false;
  bool _streamConnected = false;

  @override
  void dispose() {
    stop();
    super.dispose();
  }

  void start() {
    if (_started) return;
    _started = true;
    _startPolling();
  }

  void stop() {
    _started = false;
    _pollTimer?.cancel();
    _pollTimer = null;
  }

  void setStreamConnected(bool connected, {String? hint}) {
    _streamConnected = connected;
    _snapshot = _snapshot.copyWith(
      streamConnected: connected,
      statusHint: hint ?? (connected ? '实时流在线（轮询兜底开启）' : '实时流离线（轮询兜底开启）'),
    );
    notifyListeners();
  }

  Future<void> _startPolling() async {
    await refresh();

    final settings = await AppSettings.fromPrefs();
    final intervalSec = settings.progressIntervalSec.clamp(3, 120);

    _pollTimer?.cancel();
    _pollTimer = Timer.periodic(Duration(seconds: intervalSec), (_) {
      refresh();
    });
  }

  Future<void> refresh() async {
    _snapshot = _snapshot.copyWith(loading: true);
    notifyListeners();

    try {
      final settings = await AppSettings.fromPrefs();
      final lastTaskId = await _client.loadLastTaskId();
      var activeTaskId = lastTaskId;

      final metrics = await _client.fetchSystemMetrics(
        window: '1h',
        stepSec: settings.progressIntervalSec.clamp(1, 300),
      );

      var summary = '暂无任务，先去聊天页发送任务。';
      var status = 'idle';
      var phase = 'idle';
      var progress = 0.0;
      var updatedAt = DateTime.now();

      if (lastTaskId.isNotEmpty) {
        try {
          final task = await _client.fetchTaskProgress(lastTaskId);
          progress = _toDouble(task['percent']);
          status = (task['status'] as String?) ?? status;
          phase = (task['phase'] as String?) ?? status;
          summary = (task['summary'] as String?) ?? summary;
          final updatedAtRaw = (task['updated_at'] as String?) ?? '';
          updatedAt = DateTime.tryParse(updatedAtRaw) ?? DateTime.now();
        } catch (_) {
          await _client.clearLastTaskId();
          activeTaskId = '';
        }
      }

      final appMetrics = AppMetrics(
        cpu: _extractSeries(metrics['cpu']),
        ram: _extractSeries(metrics['ram']),
        rom: _extractSeries(metrics['rom']),
      );

      _snapshot = _snapshot.copyWith(
        taskId: activeTaskId,
        progress: progress,
        status: status,
        phase: phase,
        summary: summary,
        updatedAt: updatedAt,
        metrics: appMetrics,
        streamConnected: _streamConnected,
        statusHint: _streamConnected ? '实时流在线（轮询已同步）' : '轮询已同步（实时流离线）',
        loading: false,
        clearError: true,
      );
      notifyListeners();
    } catch (e) {
      _snapshot = _snapshot.copyWith(
        statusHint: '拉取失败：$e',
        loading: false,
        error: e,
      );
      notifyListeners();
    }
  }

  void applyStreamEvent(Map<String, dynamic> event) {
    final eventName = (event['event'] ?? event['type'] ?? '').toString();
    final dynamic rawPayload = event['data'] ?? event['payload'];
    final eventTs =
        DateTime.tryParse((event['ts'] ?? '').toString()) ?? DateTime.now();

    if (eventName.isEmpty || rawPayload is! Map<String, dynamic>) {
      return;
    }

    switch (eventName) {
      case 'task.progress':
        final taskPayload = (rawPayload['task'] is Map<String, dynamic>)
            ? (rawPayload['task'] as Map<String, dynamic>)
            : rawPayload;
        _applyTaskPayload(taskPayload);
        _snapshot = _snapshot.copyWith(
          streamConnected: _streamConnected,
          lastEventAt: eventTs,
          statusHint: _streamConnected ? '实时流在线' : _snapshot.statusHint,
          loading: false,
        );
        notifyListeners();
        break;
      case 'task.summary':
        final summary = (rawPayload['summary'] ?? '').toString();
        final status = (rawPayload['status'] ?? _snapshot.status).toString();
        final phase = (rawPayload['phase'] ?? status).toString();
        final percent = _toDouble(rawPayload['percent']);
        final hasProgress = rawPayload['percent'] is num;
        if (summary.trim().isEmpty && !hasProgress) return;
        _snapshot = _snapshot.copyWith(
          summary: summary.trim().isEmpty ? _snapshot.summary : summary,
          status: status,
          phase: phase,
          progress: hasProgress ? percent : _snapshot.progress,
          streamConnected: _streamConnected,
          lastEventAt: eventTs,
          statusHint: _streamConnected ? '实时摘要已更新' : _snapshot.statusHint,
          loading: false,
        );
        notifyListeners();
        break;
      case 'system.metrics':
        _snapshot = _snapshot.copyWith(
          streamConnected: _streamConnected,
          lastEventAt: eventTs,
          metrics: _snapshot.metrics.copyWith(
            cpu: _extractSeries(rawPayload['cpu']),
            ram: _extractSeries(rawPayload['ram']),
            rom: _extractSeries(rawPayload['rom']),
          ),
          statusHint: _streamConnected ? '实时指标已更新' : _snapshot.statusHint,
          loading: false,
        );
        notifyListeners();
        break;
      case 'chat.delta':
        final delta = (rawPayload['text'] ?? '').toString();
        if (delta.trim().isEmpty) return;
        final status = (rawPayload['status'] ?? _snapshot.status).toString();
        final phase = (rawPayload['phase'] ?? status).toString();
        final hasProgress = rawPayload['percent'] is num;
        _snapshot = _snapshot.copyWith(
          status: status,
          phase: phase,
          progress: hasProgress
              ? _toDouble(rawPayload['percent'])
              : _snapshot.progress,
          streamConnected: _streamConnected,
          lastEventAt: eventTs,
          statusHint: delta,
        );
        notifyListeners();
        break;
      default:
        break;
    }
  }

  void _applyTaskPayload(Map<String, dynamic> task) {
    final payloadTaskId = (task['task_id'] ?? '').toString();

    var nextTaskId = _snapshot.taskId;
    if (payloadTaskId.isNotEmpty) {
      nextTaskId = payloadTaskId;
    }

    final nextProgress = _toDouble(task['percent']);
    final nextStatus = (task['status'] as String?) ?? _snapshot.status;
    final nextPhase = (task['phase'] as String?) ?? nextStatus;

    var nextSummary = _snapshot.summary;
    final summary = (task['summary'] as String?) ?? '';
    if (summary.trim().isNotEmpty) {
      nextSummary = summary;
    }

    final updatedAtRaw = (task['updated_at'] as String?) ?? '';
    final nextUpdatedAt = DateTime.tryParse(updatedAtRaw) ?? DateTime.now();

    _snapshot = _snapshot.copyWith(
      taskId: nextTaskId,
      progress: nextProgress,
      status: nextStatus,
      phase: nextPhase,
      summary: nextSummary,
      updatedAt: nextUpdatedAt,
    );
  }

  List<double> _extractSeries(dynamic raw) {
    if (raw is! List) {
      return <double>[];
    }

    final values = raw
        .map((item) {
          if (item is Map<String, dynamic>) {
            return _toDouble(item['value']);
          }
          return 0.0;
        })
        .whereType<double>()
        .toList();

    if (values.length <= 24) {
      return values;
    }
    return values.sublist(values.length - 24);
  }

  double _toDouble(dynamic value) {
    if (value is num) {
      return value.toDouble();
    }
    return 0.0;
  }
}
