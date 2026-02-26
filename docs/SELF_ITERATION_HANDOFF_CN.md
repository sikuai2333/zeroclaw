# ZeroClaw 自我迭代执行链路（部署记录）

日期：2026-02-26  
执行者：Codex

## 1. 当前源码位置

- 仓库路径：`/home/sikuai/zeroclaw`
- 远端仓库：`git@github.com:sikuai2333/zeroclaw.git`
- 分支：`main`

## 2. “自我迭代”不是单点功能，而是控制平面 + 调度 + 多代理协作

核心由三部分组成：

1. 持续运行入口（systemd 用户服务）
- 保证机器开机后、即使不登录图形桌面也持续运行。
- 依赖用户 linger 已启用。

2. 控制平面（workspace 文件）
- `~/.zeroclaw/workspace/GOAL.md`：长期目标。
- `~/.zeroclaw/workspace/TASK_BOARD.md`：任务看板（待办/进行中/已完成）。
- `~/.zeroclaw/workspace/NEXT_ACTION.md`：下一步动作（单步聚焦）。
- `~/.zeroclaw/workspace/RUN_LOG.md`：执行留痕。

3. 自动调度与执行
- cron/autopilot 周期触发任务执行。
- 启用 delegate agent（研究/编码/审查）分工，形成“执行→复盘→下一步”闭环。

## 3. 已注入的仓库任务

控制平面已写入以下目标：

- 审查仓库：`/home/sikuai/zeroclaw`
- 输出要求：按优先级列问题，附文件+行号，便于 Telegram 直接追问。

## 4. 你可以如何验证

在 Ubuntu 上执行：

```bash
systemctl --user status zeroclaw-agent.service
cat ~/.zeroclaw/workspace/NEXT_ACTION.md
cat ~/.zeroclaw/workspace/TASK_BOARD.md
tail -n 50 ~/.zeroclaw/workspace/RUN_LOG.md
```

如果服务活跃、控制平面持续更新、TG 能收到周期状态，则说明“自我迭代链路”正常。

## 5. 备注

这份文档用于记录本次 0→1 基线能力，不限制你后续扩展：
- 增加更多 agent 角色
- 增加 MCP 工具
- 引入更严格的审查标准与回归测试门禁
