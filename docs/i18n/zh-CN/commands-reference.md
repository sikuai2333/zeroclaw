# ZeroClaw 命令参考（简体中文）

本页由当前 CLI 表面（`zeroclaw --help`）整理而成。

最后校验：**2026-02-26**。

> 说明：命令名、参数名、配置键保持英文；本页仅汉化说明文案。

## 顶级命令

| 命令 | 用途 |
|---|---|
| `onboard` | 初始化工作区与配置（快速/交互） |
| `agent` | 运行交互式聊天或单消息模式 |
| `gateway` | 启动 webhook 与 HTTP 网关 |
| `daemon` | 启动长期运行时（gateway + channels + heartbeat/scheduler） |
| `service` | 管理用户级系统服务 |
| `doctor` | 运行诊断与新鲜度检查 |
| `status` | 输出当前配置与系统摘要 |
| `estop` | 紧急停止（开启/恢复）与状态查询 |
| `cron` | 管理计划任务 |
| `models` | 刷新 Provider 模型目录 |
| `providers` | 查看 Provider ID、别名与当前 Provider |
| `channel` | 管理通信通道与健康检查 |
| `integrations` | 查看集成信息 |
| `skills` | 列出/安装/移除技能 |
| `migrate` | 从外部运行时迁移（当前支持 OpenClaw） |
| `config` | 导出机器可读的配置 Schema |
| `completions` | 生成 shell 自动补全脚本 |
| `hardware` | 发现并探测 USB 硬件 |
| `peripheral` | 外设配置与刷写 |

## 命令分组

### `onboard`

- `zeroclaw onboard`
- `zeroclaw onboard --interactive`
- `zeroclaw onboard --channels-only`
- `zeroclaw onboard --force`
- `zeroclaw onboard --api-key <KEY> --provider <ID> --memory <sqlite|lucid|markdown|none>`
- `zeroclaw onboard --api-key <KEY> --provider <ID> --model <MODEL_ID> --memory <sqlite|lucid|markdown|none>`

行为说明：

- 若 `config.toml` 已存在，交互模式会提供两种路径：完整覆盖 / 仅更新 Provider。
- 非交互模式下，已有 `config.toml` 时默认拒绝覆盖，除非传入 `--force`。
- 仅轮换 channel token/allowlist 时，建议使用 `--channels-only`。

### `agent`

- `zeroclaw agent`
- `zeroclaw agent -m "Hello"`
- `zeroclaw agent --provider <ID> --model <MODEL> --temperature <0.0-2.0>`
- `zeroclaw agent --peripheral <board:path>`

提示：

- 在交互会话中可自然语言要求路由切换（例如 coding 与 conversation 使用不同模型）。

### `gateway` / `daemon`

- `zeroclaw gateway [--host <HOST>] [--port <PORT>]`
- `zeroclaw daemon [--host <HOST>] [--port <PORT>]`

### `estop`

- `zeroclaw estop`
- `zeroclaw estop --level network-kill`
- `zeroclaw estop --level domain-block --domain "*.example.com"`
- `zeroclaw estop --level tool-freeze --tool shell`
- `zeroclaw estop status`
- `zeroclaw estop resume`
- `zeroclaw estop resume --otp <123456>`

注意：

- 需要 `[security.estop].enabled = true`。
- 若要求 OTP 恢复，未传 `--otp` 时会进入交互验证。

### `service`

- `zeroclaw service install`
- `zeroclaw service start`
- `zeroclaw service stop`
- `zeroclaw service restart`
- `zeroclaw service status`
- `zeroclaw service uninstall`

### `cron`

- `zeroclaw cron list`
- `zeroclaw cron add <expr> [--tz <IANA_TZ>] <command>`
- `zeroclaw cron add-at <rfc3339_timestamp> <command>`
- `zeroclaw cron add-every <every_ms> <command>`
- `zeroclaw cron once <delay> <command>`
- `zeroclaw cron remove <id>`
- `zeroclaw cron pause <id>`
- `zeroclaw cron resume <id>`

注意：

- 涉及修改调度的动作需 `cron.enabled = true`。
- 创建任务时的 shell payload 会先经过安全策略校验。

### `models`

- `zeroclaw models refresh`
- `zeroclaw models refresh --provider <ID>`
- `zeroclaw models refresh --force`

### `doctor`

- `zeroclaw doctor`
- `zeroclaw doctor models [--provider <ID>] [--use-cache]`
- `zeroclaw doctor traces [--limit <N>] [--event <TYPE>] [--contains <TEXT>]`
- `zeroclaw doctor traces --id <TRACE_ID>`

### `channel`

- `zeroclaw channel list`
- `zeroclaw channel start`
- `zeroclaw channel doctor`
- `zeroclaw channel bind-telegram <IDENTITY>`
- `zeroclaw channel add <type> <json>`
- `zeroclaw channel remove <name>`

运行期聊天命令（channel server 启动后可用）：

- 会话路由（Telegram/Discord）：
  - `/models`
  - `/models <provider>`
  - `/model`
  - `/model <model-id>`
  - `/new`
- 受监管工具授权（非 CLI channel）：
  - `/approve-request <tool-name>`：创建待确认授权请求
  - `/approve-confirm <request-id>`：确认请求（必须同一发送者 + 同一会话）
  - `/approve-pending`：列出当前范围待确认请求
  - `/approve <tool-name>`：一步授权并持久化到 `autonomy.auto_approve`
  - `/unapprove <tool-name>`：撤销授权并从 `autonomy.auto_approve` 移除
  - `/approvals`：查看运行时 + 持久化授权状态

自然语言授权策略：

- 由 `[autonomy].non_cli_natural_language_approval_mode` 控制：
  - `direct`（默认）：自然语言授权立即生效
  - `request_confirm`：自然语言先创建请求，再用 request ID 确认
  - `disabled`：忽略自然语言授权，仅接受斜杠命令
- 可使用 `[autonomy].non_cli_natural_language_approval_mode_by_channel` 做按通道覆盖。

### `integrations`

- `zeroclaw integrations info <name>`

### `skills`

- `zeroclaw skills list`
- `zeroclaw skills audit <source_or_name>`
- `zeroclaw skills install <source>`
- `zeroclaw skills remove <name>`

`<source>` 支持 git remote（`https://`、`ssh://`、`git@host:owner/repo.git`）或本地路径。

### `migrate`

- `zeroclaw migrate openclaw [--source <path>] [--dry-run]`

### `config`

- `zeroclaw config schema`

### `completions`

- `zeroclaw completions bash`
- `zeroclaw completions fish`
- `zeroclaw completions zsh`
- `zeroclaw completions powershell`
- `zeroclaw completions elvish`

### `hardware`

- `zeroclaw hardware discover`
- `zeroclaw hardware introspect <path>`
- `zeroclaw hardware info [--chip <chip_name>]`

### `peripheral`

- `zeroclaw peripheral list`
- `zeroclaw peripheral add <board> <path>`
- `zeroclaw peripheral flash [--port <serial_port>]`
- `zeroclaw peripheral setup-uno-q [--host <ip_or_host>]`
- `zeroclaw peripheral flash-nucleo`

---

如需英文原文，请查看：`docs/commands-reference.md`。
