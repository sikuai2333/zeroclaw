import 'package:flutter/material.dart';

import 'pages/chat_page.dart';
import 'pages/dashboard_page.dart';
import 'pages/settings_page.dart';

void main() {
  runApp(const ZeroClawApp());
}

class ZeroClawApp extends StatelessWidget {
  const ZeroClawApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'ZeroClaw App Channel',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF0F766E)),
        useMaterial3: true,
      ),
      home: const RootShell(),
    );
  }
}

class RootShell extends StatefulWidget {
  const RootShell({super.key});

  @override
  State<RootShell> createState() => _RootShellState();
}

class _RootShellState extends State<RootShell> {
  int _index = 0;

  @override
  Widget build(BuildContext context) {
    final pages = <Widget>[const DashboardPage(), const ChatPage()];

    final titles = ['任务总览', '聊天'];

    return Scaffold(
      appBar: AppBar(
        title: Text(titles[_index]),
        actions: [
          if (_index == 0)
            IconButton(
              icon: const Icon(Icons.settings_outlined),
              tooltip: '设置',
              onPressed: () {
                Navigator.of(context).push(
                  MaterialPageRoute<void>(builder: (_) => const SettingsPage()),
                );
              },
            ),
        ],
      ),
      body: SafeArea(
        child: IndexedStack(index: _index, children: pages),
      ),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _index,
        onDestinationSelected: (value) => setState(() => _index = value),
        destinations: const [
          NavigationDestination(
            icon: Icon(Icons.home_outlined),
            selectedIcon: Icon(Icons.home),
            label: '首页',
          ),
          NavigationDestination(
            icon: Icon(Icons.chat_bubble_outline),
            selectedIcon: Icon(Icons.chat_bubble),
            label: '聊天',
          ),
        ],
      ),
    );
  }
}
