import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import 'secure_store.dart';
import 'ws_channel_connect.dart';

class AppSettings {
  const AppSettings({
    required this.channelUrl,
    required this.channelKey,
    required this.channelId,
    required this.progressIntervalSec,
    required this.summaryIntervalSec,
  });

  final String channelUrl;
  final String channelKey;
  final String channelId;
  final int progressIntervalSec;
  final int summaryIntervalSec;

  static Future<AppSettings> fromPrefs() async {
    final prefs = await SharedPreferences.getInstance();

    // Migrate legacy plain-text key from SharedPreferences to secure storage.
    await SecureStore.migrateFromPrefsIfNeeded(
      prefsChannelKey: prefs.getString(SecureStore.channelKey),
      prefsLegacyApiKey: prefs.getString(SecureStore.legacyApiKey),
    );

    final secureKey = await SecureStore.readChannelKey();

    // Cleanup: once we have a secure key, remove legacy plain-text values.
    if (secureKey.trim().isNotEmpty) {
      if ((prefs.getString(SecureStore.channelKey) ?? '').trim().isNotEmpty) {
        await prefs.remove(SecureStore.channelKey);
      }
      if ((prefs.getString(SecureStore.legacyApiKey) ?? '').trim().isNotEmpty) {
        await prefs.remove(SecureStore.legacyApiKey);
      }
    }

    return AppSettings(
      channelUrl: prefs.getString('channel_url') ?? '',
      channelKey: secureKey,
      channelId: prefs.getString('channel_id') ?? 'app-channel-main',
      progressIntervalSec: prefs.getInt('progress_interval_sec') ?? 10,
      summaryIntervalSec: prefs.getInt('summary_interval_sec') ?? 60,
    );
  }
}

class AppChannelClient {
  const AppChannelClient();

  static const String _lastTaskIdKey = 'last_task_id';

  Future<Map<String, dynamic>> sendMessage({
    required String content,
    String sessionId = 'mobile-session',
    String userId = 'mobile-user',
  }) async {
    final settings = await AppSettings.fromPrefs();
    final uri = _buildHttpUri(settings.channelUrl, '/messages');

    final response = await http.post(
      uri,
      headers: _headers(settings),
      body: jsonEncode(<String, dynamic>{
        'session_id': sessionId,
        'user_id': userId,
        'content': content,
      }),
    );

    final data = _decodeBody(response.body);
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw Exception(
        data['error'] ?? 'message submit failed (${response.statusCode})',
      );
    }

    return data;
  }

  Future<Map<String, dynamic>> fetchTaskProgress(String taskId) async {
    final settings = await AppSettings.fromPrefs();
    final uri = _buildHttpUri(settings.channelUrl, '/tasks/$taskId/progress');

    final response = await http.get(uri, headers: _headers(settings));
    final data = _decodeBody(response.body);

    if (response.statusCode == 404) {
      throw Exception('task not found: $taskId');
    }
    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw Exception(
        data['error'] ?? 'progress request failed (${response.statusCode})',
      );
    }

    return data;
  }

  Future<Map<String, dynamic>> fetchSystemMetrics({
    String window = '1h',
    int stepSec = 10,
  }) async {
    final settings = await AppSettings.fromPrefs();
    final uri = _buildHttpUri(
      settings.channelUrl,
      '/system/metrics',
      query: <String, String>{'window': window, 'step_sec': stepSec.toString()},
    );

    final response = await http.get(uri, headers: _headers(settings));
    final data = _decodeBody(response.body);

    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw Exception(
        data['error'] ?? 'metrics request failed (${response.statusCode})',
      );
    }

    return data;
  }

  Future<WebSocketChannel> connectEventChannel() async {
    final settings = await AppSettings.fromPrefs();
    final uri = _buildWebSocketUri(
      settings.channelUrl,
      '/stream',
      query: <String, String>{
        'progress_interval_sec': settings.progressIntervalSec
            .clamp(3, 60)
            .toString(),
        'summary_interval_sec': settings.summaryIntervalSec
            .clamp(10, 300)
            .toString(),
      },
    );

    final headers = <String, dynamic>{};
    final key = settings.channelKey.trim();
    if (key.isNotEmpty) {
      headers['X-Channel-Key'] = key;
    }

    return connectWs(uri, headers: headers);
  }

  Stream<Map<String, dynamic>> eventStream(WebSocketChannel channel) {
    return channel.stream
        .map(_decodeEvent)
        .where((event) => event.isNotEmpty)
        .cast<Map<String, dynamic>>();
  }

  Future<void> closeEventChannel(WebSocketChannel channel) async {
    await channel.sink.close();
  }

  Future<void> saveLastTaskId(String taskId) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_lastTaskIdKey, taskId);
  }

  Future<String> loadLastTaskId() async {
    final prefs = await SharedPreferences.getInstance();
    return prefs.getString(_lastTaskIdKey) ?? '';
  }

  Future<void> clearLastTaskId() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(_lastTaskIdKey);
  }

  Uri _buildHttpUri(
    String rawBase,
    String suffix, {
    Map<String, String>? query,
  }) {
    final root = _apiRoot(rawBase);
    final uri = Uri.parse('$root$suffix');
    if (query == null || query.isEmpty) {
      return uri;
    }
    return uri.replace(queryParameters: query);
  }

  Uri _buildWebSocketUri(
    String rawBase,
    String suffix, {
    Map<String, String>? query,
  }) {
    final root = _apiRoot(rawBase);
    final httpUri = Uri.parse('$root$suffix');
    final wsScheme = httpUri.scheme == 'https' ? 'wss' : 'ws';
    return httpUri.replace(scheme: wsScheme, queryParameters: query);
  }

  String _apiRoot(String rawBase) {
    final base = rawBase.trim();
    if (base.isEmpty) {
      throw Exception('channel_url 未配置');
    }

    final normalized = base.endsWith('/')
        ? base.substring(0, base.length - 1)
        : base;
    return normalized.endsWith('/api/v1/app-channel')
        ? normalized
        : '$normalized/api/v1/app-channel';
  }

  Map<String, String> _headers(AppSettings settings) {
    final headers = <String, String>{
      'Content-Type': 'application/json',
      'Accept': 'application/json',
    };

    final key = settings.channelKey.trim();
    if (key.isNotEmpty) {
      headers['X-Channel-Key'] = key;
    }
    return headers;
  }

  Map<String, dynamic> _decodeBody(String body) {
    if (body.trim().isEmpty) {
      return <String, dynamic>{};
    }
    final decoded = jsonDecode(body);
    if (decoded is Map<String, dynamic>) {
      return decoded;
    }
    return <String, dynamic>{'data': decoded};
  }

  Map<String, dynamic> _decodeEvent(dynamic raw) {
    try {
      if (raw is String) {
        final decoded = jsonDecode(raw);
        if (decoded is Map<String, dynamic>) {
          return decoded;
        }
      }
      if (raw is List<int>) {
        final decoded = jsonDecode(utf8.decode(raw));
        if (decoded is Map<String, dynamic>) {
          return decoded;
        }
      }
    } catch (_) {
      // swallow decode noise and let caller keep stream alive
    }
    return <String, dynamic>{};
  }
}
