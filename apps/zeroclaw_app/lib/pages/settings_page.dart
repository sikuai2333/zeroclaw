import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../services/secure_store.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  final _urlController = TextEditingController();
  final _channelKeyController = TextEditingController();
  final _channelIdController = TextEditingController();
  final _progressIntervalController = TextEditingController();
  final _summaryIntervalController = TextEditingController();

  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void dispose() {
    _urlController.dispose();
    _channelKeyController.dispose();
    _channelIdController.dispose();
    _progressIntervalController.dispose();
    _summaryIntervalController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('设置')),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : ListView(
              padding: const EdgeInsets.all(16),
              children: [
                _buildField(
                  controller: _urlController,
                  label: 'Channel URL',
                  hint: '例如：https://example.com',
                  keyboardType: TextInputType.url,
                ),
                const SizedBox(height: 12),
                _buildField(
                  controller: _channelKeyController,
                  label: 'Channel Key（敏感）',
                  hint: '仅保存在安全存储中，不落 prefs 明文',
                  obscure: true,
                ),
                const SizedBox(height: 12),
                _buildField(
                  controller: _channelIdController,
                  label: 'Channel ID',
                  hint: '默认 app-channel-main',
                ),
                const SizedBox(height: 12),
                _buildField(
                  controller: _progressIntervalController,
                  label: '进度轮询间隔（秒）',
                  hint: '默认 10',
                  keyboardType: TextInputType.number,
                ),
                const SizedBox(height: 12),
                _buildField(
                  controller: _summaryIntervalController,
                  label: '摘要轮询间隔（秒）',
                  hint: '默认 60',
                  keyboardType: TextInputType.number,
                ),
                const SizedBox(height: 16),
                Row(
                  children: [
                    Expanded(
                      child: FilledButton.icon(
                        onPressed: _save,
                        icon: const Icon(Icons.save_outlined),
                        label: const Text('保存设置'),
                      ),
                    ),
                  ],
                ),
              ],
            ),
    );
  }

  Widget _buildField({
    required TextEditingController controller,
    required String label,
    required String hint,
    bool obscure = false,
    TextInputType keyboardType = TextInputType.text,
  }) {
    return TextField(
      controller: controller,
      obscureText: obscure,
      keyboardType: keyboardType,
      decoration: InputDecoration(
        labelText: label,
        hintText: hint,
        border: const OutlineInputBorder(),
      ),
    );
  }

  Future<void> _load() async {
    final prefs = await SharedPreferences.getInstance();

    // Best-effort migrate legacy key stored in prefs (plain-text) to secure.
    await SecureStore.migrateFromPrefsIfNeeded(
      prefsChannelKey: prefs.getString(SecureStore.channelKey),
      prefsLegacyApiKey: prefs.getString(SecureStore.legacyApiKey),
    );

    _urlController.text = prefs.getString('channel_url') ?? '';
    _channelKeyController.text = await SecureStore.readChannelKey();
    _channelIdController.text = prefs.getString('channel_id') ?? '';
    _progressIntervalController.text =
        (prefs.getInt('progress_interval_sec') ?? 10).toString();
    _summaryIntervalController.text =
        (prefs.getInt('summary_interval_sec') ?? 60).toString();

    if (!mounted) return;
    setState(() => _loading = false);
  }

  Future<void> _save() async {
    final prefs = await SharedPreferences.getInstance();

    await prefs.setString('channel_url', _urlController.text.trim());

    // Store sensitive key in secure storage.
    await SecureStore.writeChannelKey(_channelKeyController.text);

    // Cleanup legacy plain-text values (if any).
    await prefs.remove(SecureStore.channelKey);
    await prefs.remove(SecureStore.legacyApiKey);

    await prefs.setString('channel_id', _channelIdController.text.trim());
    await prefs.setInt(
      'progress_interval_sec',
      int.tryParse(_progressIntervalController.text.trim()) ?? 10,
    );
    await prefs.setInt(
      'summary_interval_sec',
      int.tryParse(_summaryIntervalController.text.trim()) ?? 60,
    );

    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('设置已保存')),
    );
  }
}
