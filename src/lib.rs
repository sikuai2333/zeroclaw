#![warn(clippy::all, clippy::pedantic)]
#![forbid(unsafe_code)]
#![allow(
    clippy::assigning_clones,
    clippy::bool_to_int_with_if,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_wrap,
    clippy::doc_markdown,
    clippy::field_reassign_with_default,
    clippy::float_cmp,
    clippy::implicit_clone,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::manual_let_else,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::new_without_default,
    clippy::needless_pass_by_value,
    clippy::needless_raw_string_hashes,
    clippy::redundant_closure_for_method_calls,
    clippy::return_self_not_must_use,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::struct_field_names,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unnecessary_cast,
    clippy::unnecessary_lazy_evaluations,
    clippy::unnecessary_literal_bound,
    clippy::unnecessary_map_or,
    clippy::unused_self,
    clippy::cast_precision_loss,
    clippy::unnecessary_wraps,
    dead_code
)]

use clap::Subcommand;
use serde::{Deserialize, Serialize};

pub mod agent;
pub(crate) mod approval;
pub(crate) mod auth;
pub mod channels;
pub mod config;
pub mod coordination;
pub(crate) mod cost;
pub(crate) mod cron;
pub(crate) mod daemon;
pub(crate) mod doctor;
pub mod gateway;
pub mod goals;
pub(crate) mod hardware;
pub(crate) mod health;
pub(crate) mod heartbeat;
pub mod hooks;
pub(crate) mod identity;
pub(crate) mod integrations;
pub mod memory;
pub(crate) mod migration;
pub(crate) mod multimodal;
pub mod observability;
pub(crate) mod onboard;
pub mod peripherals;
pub mod providers;
pub mod rag;
pub mod runtime;
pub(crate) mod security;
pub(crate) mod service;
pub(crate) mod skills;
pub mod tools;
pub(crate) mod tunnel;
pub(crate) mod util;

pub use config::Config;

/// 服务管理子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceCommands {
    /// 安装 daemon 服务单元（用于自启动与自动重启）
    Install,
    /// 启动 daemon 服务
    Start,
    /// 停止 daemon 服务
    Stop,
    /// 重启 daemon 服务并应用最新配置
    Restart,
    /// 查看 daemon 服务状态
    Status,
    /// 卸载 daemon 服务单元
    Uninstall,
}

/// Channel 管理子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelCommands {
    /// 列出所有已配置 channels
    List,
    /// 启动所有已配置 channels（异步逻辑在 main.rs）
    Start,
    /// 对已配置 channels 运行健康检查（异步逻辑在 main.rs）
    Doctor,
    /// 添加新的 channel 配置
    #[command(long_about = "\
添加新的 channel 配置。

传入 channel 类型以及该类型所需配置键的 JSON 对象。

支持类型：telegram、discord、slack、whatsapp、matrix、imessage、email。

示例：
  zeroclaw channel add telegram '{\"bot_token\":\"...\",\"name\":\"my-bot\"}'
  zeroclaw channel add discord '{\"bot_token\":\"...\",\"name\":\"my-discord\"}'")]
    Add {
        /// Channel 类型（telegram, discord, slack, whatsapp, matrix, imessage, email）
        channel_type: String,
        /// JSON 格式配置
        config: String,
    },
    /// 移除 channel 配置
    Remove {
        /// 待移除的 channel 名称
        name: String,
    },
    /// 将 Telegram 身份（用户名或数字 ID）加入 allowlist
    #[command(long_about = "\
将 Telegram 身份加入 allowlist。

支持 Telegram 用户名（不含 '@'）或数字用户 ID。\
加入后 agent 会响应该身份发送的消息。

示例：
  zeroclaw channel bind-telegram zeroclaw_user
  zeroclaw channel bind-telegram 123456789")]
    BindTelegram {
        /// 允许的 Telegram 身份（用户名不含 '@'，或数字用户 ID）
        identity: String,
    },
}

/// Skills 管理子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillCommands {
    /// 列出所有已安装 skills
    List,
    /// 审计 skill 源目录或已安装 skill 名称
    Audit {
        /// Skill 路径或已安装 skill 名称
        source: String,
    },
    /// 从 URL 或本地路径安装 skill
    Install {
        /// 来源 URL 或本地路径
        source: String,
    },
    /// 移除已安装 skill
    Remove {
        /// 待移除 skill 名称
        name: String,
    },
}

/// 迁移子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MigrateCommands {
    /// 从 `OpenClaw` 工作区导入 memory 到当前 `ZeroClaw` 工作区
    Openclaw {
        /// `OpenClaw` 工作区路径（默认 ~/.openclaw/workspace）
        #[arg(long)]
        source: Option<std::path::PathBuf>,

        /// 仅校验与预览，不写入数据
        #[arg(long)]
        dry_run: bool,
    },
}

/// Cron 子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CronCommands {
    /// 列出所有计划任务
    List,
    /// 添加新的计划任务
    #[command(long_about = "\
添加新的周期计划任务。

使用标准 5 字段 cron 语法：`min hour day month weekday`。\
默认按 UTC 计算；可通过 --tz 指定 IANA 时区覆盖。

示例：
  zeroclaw cron add '0 9 * * 1-5' 'Good morning' --tz America/New_York
  zeroclaw cron add '*/30 * * * *' 'Check system health'")]
    Add {
        /// Cron 表达式
        expression: String,
        /// IANA 时区（例如 America/Los_Angeles）
        #[arg(long)]
        tz: Option<String>,
        /// 要执行的命令
        command: String,
    },
    /// 按 RFC3339 时间戳添加一次性任务
    #[command(long_about = "\
添加在指定 UTC 时间触发的一次性任务。

时间戳必须使用 RFC 3339 格式（如 2025-01-15T14:00:00Z）。

示例：
  zeroclaw cron add-at 2025-01-15T14:00:00Z 'Send reminder'
  zeroclaw cron add-at 2025-12-31T23:59:00Z 'Happy New Year!'")]
    AddAt {
        /// RFC3339 格式时间戳
        at: String,
        /// 要执行的命令
        command: String,
    },
    /// 添加固定间隔任务
    #[command(long_about = "\
添加按固定间隔重复执行的任务。

间隔单位为毫秒，例如 60000 = 1 分钟。

示例：
  zeroclaw cron add-every 60000 'Ping heartbeat'     # every minute
  zeroclaw cron add-every 3600000 'Hourly report'    # every hour")]
    AddEvery {
        /// 间隔（毫秒）
        every_ms: u64,
        /// 要执行的命令
        command: String,
    },
    /// 添加一次性延迟任务（如 "30m"、"2h"、"1d"）
    #[command(long_about = "\
添加“从现在起延迟触发”的一次性任务。

支持可读时长：s（秒）、m（分）、h（时）、d（天）。

示例：
  zeroclaw cron once 30m 'Run backup in 30 minutes'
  zeroclaw cron once 2h 'Follow up on deployment'
  zeroclaw cron once 1d 'Daily check'")]
    Once {
        /// 延迟时长
        delay: String,
        /// 要执行的命令
        command: String,
    },
    /// 删除计划任务
    Remove {
        /// 任务 ID
        id: String,
    },
    /// 更新计划任务
    #[command(long_about = "\
更新已有任务的一个或多个字段。

仅会修改你指定的字段，其他字段保持不变。

示例：
  zeroclaw cron update <task-id> --expression '0 8 * * *'
  zeroclaw cron update <task-id> --tz Europe/London --name 'Morning check'
  zeroclaw cron update <task-id> --command 'Updated message'")]
    Update {
        /// 任务 ID
        id: String,
        /// 新 cron 表达式
        #[arg(long)]
        expression: Option<String>,
        /// 新 IANA 时区
        #[arg(long)]
        tz: Option<String>,
        /// 新执行命令
        #[arg(long)]
        command: Option<String>,
        /// 新任务名称
        #[arg(long)]
        name: Option<String>,
    },
    /// 暂停任务
    Pause {
        /// 任务 ID
        id: String,
    },
    /// 恢复已暂停任务
    Resume {
        /// 任务 ID
        id: String,
    },
}

/// Memory 管理子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryCommands {
    /// 列出 memory 条目（可选过滤）
    List {
        /// 按 category 过滤（core、daily、conversation 或自定义）
        #[arg(long)]
        category: Option<String>,
        /// 按 session ID 过滤
        #[arg(long)]
        session: Option<String>,
        /// 最大显示条目数
        #[arg(long, default_value = "50")]
        limit: usize,
        /// 跳过条目数（分页）
        #[arg(long, default_value = "0")]
        offset: usize,
    },
    /// 按 key 获取单条 memory
    Get {
        /// 待查询的 memory key
        key: String,
    },
    /// 查看 memory backend 统计与健康状态
    Stats,
    /// 按 category / key 清理 memory，或全部清理
    Clear {
        /// 按 key 删除单条（支持前缀匹配）
        #[arg(long)]
        key: Option<String>,
        /// 仅清理指定 category
        #[arg(long)]
        category: Option<String>,
        /// 跳过确认提示
        #[arg(long)]
        yes: bool,
    },
}

/// Integration 子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntegrationCommands {
    /// 列出所有 integrations（可按 category/status 过滤）
    List {
        /// 按 category 过滤（如 "chat"、"ai"、"productivity"）
        #[arg(long, short)]
        category: Option<String>,
        /// 按状态过滤：active、available、coming-soon
        #[arg(long, short)]
        status: Option<String>,
    },
    /// 按关键字搜索 integrations（匹配名称和描述）
    Search {
        /// 搜索关键词
        query: String,
    },
    /// 查看指定 integration 详情
    Info {
        /// Integration 名称
        name: String,
    },
}

/// 硬件发现子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HardwareCommands {
    /// 枚举 USB 设备（VID/PID）并识别已知开发板
    #[command(long_about = "\
枚举 USB 设备并显示已知开发板。

按 VID/PID 扫描已连接 USB 设备，并匹配常见开发板 \
（STM32 Nucleo、Arduino、ESP32）。

示例：
  zeroclaw hardware discover")]
    Discover,
    /// 按路径探测设备（例如 /dev/ttyACM0）
    #[command(long_about = "\
按串口或设备路径探测设备。

打开指定设备路径并查询板卡信息、固件版本与支持能力。

示例：
  zeroclaw hardware introspect /dev/ttyACM0
  zeroclaw hardware introspect COM3")]
    Introspect {
        /// 串口或设备路径
        path: String,
    },
    /// 通过 USB 获取芯片信息（probe-rs over ST-Link，无需目标板固件）
    #[command(long_about = "\
通过 USB 使用 probe-rs（ST-Link）读取芯片信息。

通过调试探针直接访问目标 MCU，无需在目标板上预刷固件。

示例：
  zeroclaw hardware info
  zeroclaw hardware info --chip STM32F401RETx")]
    Info {
        /// 芯片名（如 STM32F401RETx）。默认 Nucleo-F401RE 使用 STM32F401RETx
        #[arg(long, default_value = "STM32F401RETx")]
        chip: String,
    },
}

/// 外设（硬件）管理子命令
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PeripheralCommands {
    /// 列出已配置外设
    List,
    /// 添加外设（board + path，例如 nucleo-f401re /dev/ttyACM0）
    #[command(long_about = "\
按板卡类型与传输路径添加外设。

注册硬件板卡后，agent 可调用其工具能力（GPIO、传感器、执行器）。\
单板机（如树莓派）本地 GPIO 可将路径设为 `native`。

支持板卡：nucleo-f401re、rpi-gpio、esp32、arduino-uno。

示例：
  zeroclaw peripheral add nucleo-f401re /dev/ttyACM0
  zeroclaw peripheral add rpi-gpio native
  zeroclaw peripheral add esp32 /dev/ttyUSB0")]
    Add {
        /// 板卡类型（nucleo-f401re、rpi-gpio、esp32）
        board: String,
        /// 串口路径（如 /dev/ttyACM0）或本地 GPIO 的 `native`
        path: String,
    },
    /// 将 ZeroClaw 固件刷入 Arduino（生成 .ino、按需安装 arduino-cli、编译并上传）
    #[command(long_about = "\
将 ZeroClaw 固件刷入 Arduino 板卡。

会生成 .ino、检测并安装 arduino-cli（若缺失）、编译并上传固件。

示例：
  zeroclaw peripheral flash
  zeroclaw peripheral flash --port /dev/cu.usbmodem12345
  zeroclaw peripheral flash -p COM3")]
    Flash {
        /// 串口（如 /dev/cu.usbmodem12345）；省略时使用配置中的首个 arduino-uno
        #[arg(short, long)]
        port: Option<String>,
    },
    /// 配置 Arduino Uno Q Bridge 应用（部署 GPIO bridge 供 agent 控制）
    SetupUnoQ {
        /// Uno Q IP（如 192.168.0.48）；省略时默认在 Uno Q 本机运行
        #[arg(long)]
        host: Option<String>,
    },
    /// 刷写 ZeroClaw 固件到 Nucleo-F401RE（编译 + probe-rs）
    FlashNucleo,
}
