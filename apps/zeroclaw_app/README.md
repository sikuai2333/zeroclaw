# zeroclaw_app

Flutter 客户端（App Channel）。

## 目标

- 首页（Dashboard）：展示任务总进度、阶段总结、检查点、CPU/RAM/ROM 指标图。
- 聊天页（Chat）：发送消息到 App Channel，获得 task_id。
- 设置页（Settings）：配置 Channel URL 与 `X-Channel-Key`（可选）。

## 关键约束（NAS）

- NAS 本机禁止 `flutter run` / `flutter build`。
- 允许：工程创建、代码编辑、`flutter analyze --no-pub` 等轻量检查。
- 依赖解析（`flutter pub get`）可在 **非 NAS** 的开发机/CI 环境执行。

## 配置

设置页会持久化以下 key（SharedPreferences）：

- `channel_url`：服务端根地址或 app-channel 根地址。
  - 允许输入：
    - `https://example.com`（客户端会自动拼接 `/api/v1/app-channel`）
    - `https://example.com/api/v1/app-channel`（已包含则不重复拼接）
- `channel_key`：用于请求头 `X-Channel-Key`（可选；后端也兼容 Bearer/Pairing 回退策略）
- `channel_id`：预留字段（当前客户端主要使用 `task_id`）
- `progress_interval_sec`：轮询刷新间隔（Dashboard 轮询 progress + metrics）
- `summary_interval_sec`：预留字段

兼容：旧版本若存过 `api_key`，客户端会回退读取。

## API（V1）

- `POST /api/v1/app-channel/messages`
- `GET /api/v1/app-channel/tasks/{task_id}/progress`
- `GET /api/v1/app-channel/system/metrics`

契约：`docs/revamp/app-channel-api-v1.yaml`

## 本地检查（轻量）

在 NAS：

```bash
cd apps/zeroclaw_app
flutter analyze --no-pub
```

在开发机（允许 pub get 的环境）：

```bash
cd apps/zeroclaw_app
flutter pub get
flutter analyze
```
