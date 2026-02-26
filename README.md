# ZeroClaw（默认中文文档）

ZeroClaw 是一个面向 Agent 工作流的运行时系统：

- 统一 Provider / Channel / Tool / Memory
- 支持常驻运行（daemon + service）
- 支持 Telegram 等通道会话
- 支持模型与工具的运行时切换

> 命令名、参数名、配置键保持英文；本 README 仅汉化说明。

## 语言入口

- 中文（默认）：`README.md`
- 英文：`docs/en/README.md`
- 中文文档中心：`docs/i18n/zh-CN/README.md`

## 快速开始

### 1. 安装依赖（Ubuntu）

```bash
sudo apt update
sudo apt install -y build-essential pkg-config curl git
```

### 2. 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version
cargo --version
```

### 3. 构建

```bash
cargo build --release
./target/release/zeroclaw --help
```

## 常用命令

```bash
# 初始化配置
zeroclaw onboard

# 启动长期运行时（推荐生产使用）
zeroclaw daemon

# 启动 channel server
zeroclaw channel start

# 查看状态
zeroclaw status

# 检查服务状态
zeroclaw service status
```

## Telegram 会话内常用命令

以下命令在 channel 运行时可用（命令本体不翻译）：

- `/models`
- `/models <provider>`
- `/model <model-id>`
- `/new`
- `/approve-request <tool-name>`
- `/approve-confirm <request-id>`
- `/approve-pending`
- `/approve <tool-name>`
- `/unapprove <tool-name>`
- `/approvals`

## 文档导航

- 命令参考（中文）：`docs/i18n/zh-CN/commands-reference.md`
- 命令参考（英文）：`docs/commands-reference.md`
- 配置参考（中文）：`docs/i18n/zh-CN/config-reference.md`
- Channel 参考（中文）：`docs/i18n/zh-CN/channels-reference.md`
- 运维 Runbook（中文）：`docs/i18n/zh-CN/operations-runbook.md`

## 发布与下载

本仓库已提供 Ubuntu 构建工作流：

- `Pub Ubuntu Build`：`.github/workflows/pub-ubuntu-build.yml`

触发方式：

1. 手动触发 `workflow_dispatch`（可选发布 Release）
2. 推送 tag（`v*`）自动触发并发布

发布后可在这里下载：

- `https://github.com/sikuai2333/zeroclaw/releases`

## 说明

如果你准备进行深度改造，建议流程：

1. 先冻结主分支，开一个改造分支。
2. 先改命令面与配置面，再改执行面。
3. 每次改动保持可构建、可回滚。
