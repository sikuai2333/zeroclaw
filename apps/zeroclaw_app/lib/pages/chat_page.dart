import 'dart:async';

import 'package:flutter/material.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import '../services/app_channel_client.dart';

class ChatPage extends StatefulWidget {
  const ChatPage({super.key});

  @override
  State<ChatPage> createState() => _ChatPageState();
}

class _ChatPageState extends State<ChatPage> {
  final TextEditingController _controller = TextEditingController();
  final AppChannelClient _client = const AppChannelClient();

  final List<_ChatMessage> _messages = <_ChatMessage>[
    const _ChatMessage(
      role: 'assistant',
      text: '已连接 ZeroClaw App Channel（配置好 Channel URL / X-Channel-Key 后可发送）。',
    ),
  ];

  WebSocketChannel? _eventChannel;
  StreamSubscription<Map<String, dynamic>>? _eventSub;
  Timer? _streamRetryTimer;

  bool _sending = false;
  String _lastTaskId = '';
  bool _processing = false;
  String _processingHint = '任务处理中…';
  DateTime? _lastEventAt;
  final Map<String, DateTime> _recentEventMessages = <String, DateTime>{};
  static const Duration _eventDedupWindow = Duration(seconds: 8);

  @override
  void initState() {
    super.initState();
    _loadLastTaskId();
    _connectRealtimeStream();
  }

  @override
  void dispose() {
    _controller.dispose();
    _streamRetryTimer?.cancel();
    _eventSub?.cancel();
    final channel = _eventChannel;
    _eventChannel = null;
    if (channel != null) {
      _client.closeEventChannel(channel);
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        if (_lastTaskId.trim().isNotEmpty)
          MaterialBanner(
            content: Text('最近 task_id：$_lastTaskId'),
            leading: const Icon(Icons.track_changes_outlined),
            actions: [
              TextButton(
                onPressed: _sending
                    ? null
                    : () async {
                        await _client.clearLastTaskId();
                        if (!mounted) return;
                        setState(() => _lastTaskId = '');
                      },
                child: const Text('清除'),
              ),
            ],
          ),
        Expanded(
          child: ListView.builder(
            padding: const EdgeInsets.all(12),
            itemCount: _messages.length + ((_sending || _processing) ? 1 : 0),
            itemBuilder: (context, index) {
              if ((_sending || _processing) && index == _messages.length) {
                final typingText = _sending ? 'Agent 正在输入中…' : _processingHint;
                return _TypingBubble(text: typingText);
              }
              final message = _messages[index];
              final mine = message.role == 'user';
              return Align(
                alignment: mine ? Alignment.centerRight : Alignment.centerLeft,
                child: Container(
                  margin: const EdgeInsets.symmetric(vertical: 4),
                  padding: const EdgeInsets.all(10),
                  constraints: const BoxConstraints(maxWidth: 320),
                  decoration: BoxDecoration(
                    color: mine
                        ? Theme.of(context).colorScheme.primaryContainer
                        : Theme.of(context).colorScheme.surfaceContainerHighest,
                    borderRadius: BorderRadius.circular(12),
                  ),
                  child: Text(message.text),
                ),
              );
            },
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
          child: Row(
            children: [
              Icon(
                _eventChannel == null ? Icons.wifi_off : Icons.wifi_tethering,
                size: 16,
                color: _eventChannel == null ? Colors.grey : Colors.green,
              ),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  _lastEventAt == null
                      ? '实时流状态：等待事件'
                      : '最近事件：${DateTime.now().difference(_lastEventAt!).inSeconds} 秒前',
                  style: Theme.of(context).textTheme.bodySmall,
                ),
              ),
            ],
          ),
        ),
        SafeArea(
          top: false,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 6, 12, 12),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _controller,
                    decoration: const InputDecoration(
                      hintText: '输入任务或问题…',
                      border: OutlineInputBorder(),
                      isDense: true,
                    ),
                    onSubmitted: (_) => _send(),
                    enabled: !_sending,
                  ),
                ),
                const SizedBox(width: 8),
                FilledButton.icon(
                  onPressed: _sending ? null : _send,
                  icon: const Icon(Icons.send),
                  label: const Text('发送'),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }

  Future<void> _loadLastTaskId() async {
    final taskId = await _client.loadLastTaskId();
    if (!mounted) return;
    setState(() => _lastTaskId = taskId);
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
      _eventSub = _client
          .eventStream(channel)
          .listen(
            _onStreamEvent,
            onDone: _scheduleReconnect,
            onError: (_, __) => _scheduleReconnect(),
          );
    } catch (_) {
      _scheduleReconnect();
    }
  }

  void _scheduleReconnect() {
    final oldChannel = _eventChannel;
    _eventChannel = null;
    if (oldChannel != null) {
      unawaited(_client.closeEventChannel(oldChannel));
    }
    _streamRetryTimer?.cancel();
    _streamRetryTimer = Timer(const Duration(seconds: 5), () {
      if (!mounted) return;
      _connectRealtimeStream();
    });
  }

  void _onStreamEvent(Map<String, dynamic> event) {
    final eventName = (event['event'] ?? event['type'] ?? '').toString();
    final payload = event['data'] ?? event['payload'];
    final taskId = (event['task_id'] ?? '').toString();

    if (payload is! Map<String, dynamic>) return;
    if (_lastTaskId.isNotEmpty && taskId.isNotEmpty && taskId != _lastTaskId) {
      return;
    }

    if (eventName == 'task.progress') {
      final status = (payload['status'] ?? '').toString();
      final phase = (payload['phase'] ?? status).toString();
      final percent = (payload['percent'] is num)
          ? (payload['percent'] as num).toDouble()
          : null;
      _lastEventAt =
          DateTime.tryParse((event['ts'] ?? '').toString()) ?? DateTime.now();
      final processing = status == 'queued' || status == 'running';
      if (mounted) {
        setState(() {
          _processing = processing;
          if (processing && percent != null) {
            _processingHint =
                'Agent 正在输入中…（$status/$phase，${percent.toStringAsFixed(1)}%）';
          } else if (!processing) {
            _processingHint = '任务已结束';
          }
        });
      }
      return;
    }

    if (eventName == 'chat.delta') {
      final text = (payload['text'] ?? '').toString();
      if (text.trim().isEmpty) return;
      if (_isDuplicateEventMessage('chat.delta', text)) return;
      _lastEventAt =
          DateTime.tryParse((event['ts'] ?? '').toString()) ?? DateTime.now();
      _appendAssistant('进度更新：$text');
      return;
    }

    if (eventName == 'task.summary') {
      final summary = (payload['summary'] ?? '').toString();
      if (summary.trim().isEmpty) return;
      final status = (payload['status'] ?? '').toString();
      final phase = (payload['phase'] ?? status).toString();
      final percent = payload['percent'];
      final kind = (payload['kind'] ?? 'periodic').toString();
      final progressText = percent is num
          ? ' ${percent.toStringAsFixed(1)}%'
          : '';
      final statusText = status.trim().isEmpty
          ? ''
          : ' [$status/$phase$progressText]';
      _lastEventAt =
          DateTime.tryParse((event['ts'] ?? '').toString()) ?? DateTime.now();
      if (status == 'succeeded' || status == 'failed') {
        if (mounted) {
          setState(() {
            _processing = false;
            _sending = false;
            if (status == 'failed') {
              _processingHint = '任务失败';
            }
          });
        }
      }
      if (status == 'failed') {
        final failureReason = (payload['failure_reason'] ?? summary)
            .toString()
            .trim();
        final retrySuggestion =
            (payload['retry_suggestion'] ?? '建议检查日志后重试，或发送 /触发 获取下一步。')
                .toString()
                .trim();
        final triggerCommand = (payload['trigger_command'] ?? '/触发')
            .toString()
            .trim();
        final failureText =
            '任务失败$statusText：$failureReason\n建议：$retrySuggestion\n快捷指令：$triggerCommand';
        if (_isDuplicateEventMessage('task.summary.failed', failureText))
          return;
        _appendAssistant(failureText);
        return;
      }

      final summaryText = kind == 'final'
          ? '最终总结$statusText：$summary'
          : '阶段总结$statusText：$summary';
      if (_isDuplicateEventMessage('task.summary', summaryText)) return;
      _appendAssistant(summaryText);
    }
  }

  bool _isDuplicateEventMessage(String eventType, String text) {
    final normalized = text.trim();
    if (normalized.isEmpty) return true;

    final now = DateTime.now();
    _recentEventMessages.removeWhere(
      (_, ts) => now.difference(ts) > _eventDedupWindow,
    );
    final key = '$eventType::$normalized';
    final lastSeen = _recentEventMessages[key];
    if (lastSeen != null && now.difference(lastSeen) <= _eventDedupWindow) {
      return true;
    }
    _recentEventMessages[key] = now;
    return false;
  }

  void _appendAssistant(String text) {
    if (!mounted) return;
    setState(() {
      _sending = false;
      _messages.add(_ChatMessage(role: 'assistant', text: text));
    });
  }

  Future<void> _send() async {
    final text = _controller.text.trim();
    if (text.isEmpty) return;

    setState(() {
      _messages.add(_ChatMessage(role: 'user', text: text));
      _controller.clear();
      _sending = true;
      _processing = true;
      _processingHint = '任务已提交，等待接收确认…';
    });

    try {
      final accepted = await _client.sendMessage(content: text);
      final taskId = (accepted['task_id'] ?? '').toString();
      if (taskId.isNotEmpty) {
        await _client.saveLastTaskId(taskId);
      }

      if (!mounted) return;
      setState(() {
        _sending = false;
        _processing = true;
        _processingHint = 'Agent 正在输入中…（任务已受理）';
        _lastTaskId = taskId.isNotEmpty ? taskId : _lastTaskId;
        _messages.add(
          _ChatMessage(
            role: 'assistant',
            text: taskId.isNotEmpty
                ? '已提交（accepted=true）。task_id=$taskId\n实时进度将持续推送。'
                : '已提交（accepted=true）。未返回 task_id。',
          ),
        );
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _sending = false;
        _processing = false;
        _messages.add(_ChatMessage(role: 'assistant', text: '发送失败：$e'));
      });

      if (!mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text('发送失败：$e')));
    }
  }
}

class _ChatMessage {
  const _ChatMessage({required this.role, required this.text});

  final String role;
  final String text;
}

class _TypingBubble extends StatelessWidget {
  const _TypingBubble({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 4),
        padding: const EdgeInsets.all(10),
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(12),
        ),
        child: Text(text),
      ),
    );
  }
}
