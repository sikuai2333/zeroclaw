import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Centralized secure storage wrapper.
///
/// NOTE: Do not store sensitive fields in SharedPreferences.
class SecureStore {
  const SecureStore._();

  static const FlutterSecureStorage _storage = FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
  );

  /// Secure key name.
  static const String channelKey = 'channel_key';
  /// Backward-compat: previously used 'api_key'.
  static const String legacyApiKey = 'api_key';

  static Future<String> readChannelKey() async {
    return (await _storage.read(key: channelKey)) ??
        (await _storage.read(key: legacyApiKey)) ??
        '';
  }

  static Future<void> writeChannelKey(String value) async {
    final trimmed = value.trim();
    if (trimmed.isEmpty) {
      await _storage.delete(key: channelKey);
      return;
    }
    await _storage.write(key: channelKey, value: trimmed);
  }

  static Future<void> migrateFromPrefsIfNeeded({
    required String? prefsChannelKey,
    required String? prefsLegacyApiKey,
  }) async {
    // If secure already has a value, do nothing.
    final existing = await _storage.read(key: channelKey);
    if (existing != null && existing.trim().isNotEmpty) {
      return;
    }

    final candidate = (prefsChannelKey ?? '').trim().isNotEmpty
        ? (prefsChannelKey ?? '').trim()
        : (prefsLegacyApiKey ?? '').trim();

    if (candidate.isEmpty) return;

    await _storage.write(key: channelKey, value: candidate);
  }
}
