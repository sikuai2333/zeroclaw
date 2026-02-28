# Flutter Channel 改版总规划（NAS 版）

日期：2026-02-27  
执行者：Codex

## 1. 目标

把当前仓库升级为“可通过独立 App 渠道沟通与任务可视化”的系统：

1. 新增一个 App Channel（后端接口 + 协议 + 状态上报）。
2. 新建 Flutter 客户端工程（仅创建与代码开发，不在 NAS 本机 build/run）。
3. App 首页展示任务总进度（0-100%）、阶段总结、服务器状态图表（CPU/RAM/ROM）。
4. App 第二页为聊天页，第三页为设置页（首页右上角进入设置）。
5. GitHub Actions 改为“功能完成才构建”，小 push 不触发重编译。
6. 每次触发构建后，必须自动拉取构建结果；失败必须生成修复任务并推进修复。

## 2. 约束（强制）

1. NAS 上允许：`flutter create`、代码编辑、静态检查、轻量命令。  
2. NAS 上禁止：`flutter run`、`flutter build`、任何重资源本地编译。  
3. 所有 App 编译仅通过 GitHub Actions 执行。  
4. 非功能完成状态，不允许触发远端构建。

## 3. CI/CD 触发策略（小 push 不触发）

### 3.1 分层策略

1. **轻量检查流**（允许小 push 触发）
- 只跑：格式检查/脚本检查/文档检查/少量单测。
- 不执行 Flutter build，不执行重型 Rust 全量构建。

2. **重构建流**（仅功能完成触发）
- 仅在以下任一条件触发：
  - `workflow_dispatch` 且 `feature_ready=true`
  - push 标签 `feature/*-ready`（由完成流程创建）
  - 显式存在构建许可文件（见 3.2）

### 3.2 构建门禁文件（建议）

新增目录：`.ci/feature-gates/`  
每个功能完成时提交一个 gate 文件，例如：
- `.ci/feature-gates/app-channel-v1.yaml`

字段建议：
- `feature_id`
- `acceptance_checked: true`
- `owner`
- `ready_for_build: true`
- `timestamp`

重构建 workflow 第一阶段先校验 gate 文件；不满足则立即失败并退出（不占资源）。

### 3.3 构建结果闭环

每次重构建后自动执行：
1. 拉取 run 结果与日志（优先 `gh run view --log-failed`，空则回退 `gh api .../logs` 解压解析）。
2. 生成/更新 `CI_FAILURES.md`（根因、受影响模块、修复建议）。
3. 把修复任务写回 `TASK_BOARD.md` 与 `NEXT_ACTION.md`。

## 4. Flutter 客户端方案

目录建议：`apps/zeroclaw_app/`

### 4.1 页面结构

1. **首页（Dashboard）**
- 顶部：任务名称、当前阶段、总进度条（0~100%）
- 中间：阶段卡片（如 30/50/70/99/100）
- 下方：服务器资源图（CPU、RAM、ROM，折线/面积图）
- 右上角：设置按钮（跳转设置页）

2. **聊天页（Chat）**
- 会话列表 + 当前会话消息流
- 发送输入框
- Agent “处理中”状态
- 任务相关消息可标注到某个 task_id

3. **设置页（Settings）**
- `channel_url`
- `api_key`（安全存储）
- `channel_id` / `agent_id`
- 刷新频率（进度轮询间隔、状态轮询间隔）

### 4.2 状态更新机制

1. 任务执行中每隔 N 秒更新一次进度（建议 5~10 秒）。
2. 每隔 M 秒生成一次阶段总结（建议 60~120 秒）。
3. 总进度必须可解释：
- `percent`（0-100）
- `phase`
- `evidence`（最近动作/命令/结果）

### 4.3 图表技术选型

1. 首选 `fl_chart`（成熟、可定制、跨平台稳定）。
2. 首页至少 3 个时间序列图：CPU%、RAM%、ROM%。

## 5. 后端与接口规划（ZeroClaw 扩展）

## 5.1 新增 Channel 类型

新增：`channel.app`（HTTP + WebSocket）

能力：
1. 接收用户消息并路由到现有 agent runtime。
2. 推送任务进度事件（progress event）。
3. 推送周期性总结事件（summary event）。
4. 查询系统指标（CPU/RAM/ROM）。

## 5.2 接口草案（V1）

### 鉴权（App Channel Key）

- HTTP：优先使用 `X-Channel-Key: <raw_key>`（或 `Authorization: Bearer <raw_key>`）。
- WebSocket：浏览器端无法自定义 header，允许 `?channel_key=<raw_key>` 作为兜底。

服务端支持两种配置方式（推荐使用哈希模式，避免在服务器环境变量中保存明文）：

1) **哈希模式（推荐）**：
- `ZEROCLAW_APP_CHANNEL_KEY_SHA256`：`hex(sha256(raw_key))`（64 hex chars）

2) **明文模式（兼容）**：
- `ZEROCLAW_APP_CHANNEL_KEY`：`raw_key`

注意：鉴权失败响应不会返回任何密钥相关信息；审计日志也不会记录密钥明文。

1. `POST /api/v1/app-channel/messages`
- 入参：`session_id, user_id, content`
- 出参：`message_id, accepted`

2. `GET /api/v1/app-channel/tasks/{task_id}/progress`
- 出参：`percent, phase, updated_at, summary, checkpoints`

3. `GET /api/v1/app-channel/system/metrics?window=1h`
- 出参：CPU/RAM/ROM 时间序列

4. `WS /api/v1/app-channel/stream`
- 事件：`chat.delta`, `task.progress`, `task.summary`, `system.metrics`

## 5.3 安全基线（保留）

1. App Channel 使用 API Key + HMAC 时间戳签名（二选一可先 API Key）。
2. 服务端保存密钥哈希，不明文回显。
3. 必须有请求频率限制与基础审计日志。
4. Settings 中密钥使用平台安全存储（如 secure storage）。

## 6. 里程碑与验收

### M1：CI 门禁改造
- 完成：重构建仅功能完成触发。
- 验收：连续 3 次小 push 不触发重构建；一次 gate 触发可启动构建。

### M2：Flutter 工程骨架
- 完成：首页/聊天/设置三页路由与基础状态管理。
- 验收：页面跳转完整，设置可持久化。

### M3：App Channel 后端 V1
- 完成：消息、进度、指标、WS 推送。
- 验收：App 可收发消息并显示实时进度。

### M4：构建闭环自动修复
- 完成：每次 CI 失败自动生成修复任务并推进。
- 验收：`CI_FAILURES.md`、`RUN_LOG.md`、`NEXT_ACTION.md` 自动联动更新。

## 7. 执行策略（资源友好）

1. NAS 只做代码生成/编辑/轻量验证。  
2. 重编译全部迁移到 GitHub Actions。  
3. 每轮任务只推进一个最小可验证动作，并强制更新进度文件。  
4. 若失败，先产出可读根因，再进入修复。

## 8. 下一步（立即执行）

1. 先改 workflow 触发门禁（M1）。
2. 初始化 Flutter 工程骨架（`flutter create apps/zeroclaw_app`）。
3. 设计并落地 App Channel API 契约文件（OpenAPI 草案）。
