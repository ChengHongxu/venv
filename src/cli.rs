//! Command line. Help is in Chinese, including clap's own headings, which
//! [`command`] rewrites.

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};

use crate::config::Backend;
use crate::shell::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "venv",
    version,
    about = "管理 Python 虚拟环境，后端可选 pixi、uv、python-venv",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Set by the shell function: print shell code for it to evaluate
    #[arg(long, value_enum, global = true, hide = true)]
    pub emit: Option<Shell>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// 创建环境并激活
    New(NewArgs),
    /// 激活环境；不带名字时列出所有环境。别名 use
    #[command(alias = "use")]
    Switch(SwitchArgs),
    /// 退出当前环境
    Off,
    /// 列出所有环境
    List(ListArgs),
    /// 删除环境（声明文件和环境本体）
    Rm(RmArgs),
    /// 给环境加包（默认当前环境）
    Add(PkgArgs),
    /// 从环境删包（默认当前环境）
    Remove(PkgArgs),
    /// 按声明文件安装或更新环境，不带名字时处理全部
    Sync(SyncArgs),
    /// 在环境里运行命令，不需要激活
    Run(RunArgs),
    /// 打印环境前缀（默认当前环境）
    Path(PathArgs),
    /// 打印环境里某个命令的路径
    Which(WhichArgs),
    /// 把 shell 集成写进启动文件（~/.zshrc 或 ~/.bashrc）
    Init(InitArgs),
    /// 打印 shell 集成代码（启动文件里的 eval 行执行的就是它）
    Hook(HookArgs),
    /// 查看配置文件路径和当前生效的配置
    Config,
    /// Run by the shell integration at startup
    #[command(hide = true)]
    Startup,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    /// 环境名
    #[arg(value_name = "名字")]
    pub name: String,
    /// 后端，默认取配置里的 default_backend
    #[arg(long, value_enum)]
    pub backend: Option<Backend>,
    /// Python 版本，例如 3.12；默认取配置里的 default_python
    #[arg(long, value_name = "版本")]
    pub python: Option<String>,
    /// 创建后不激活
    #[arg(long)]
    pub no_switch: bool,
}

#[derive(Debug, Args)]
pub struct SwitchArgs {
    /// 环境名
    #[arg(value_name = "名字")]
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// 输出 JSON
    #[arg(long)]
    pub json: bool,
    /// Only the names, one per line (completion)
    #[arg(long, hide = true)]
    pub names: bool,
}

#[derive(Debug, Args)]
pub struct RmArgs {
    /// 环境名
    #[arg(value_name = "名字")]
    pub name: String,
    /// 不确认，直接删除
    #[arg(long, short)]
    pub yes: bool,
    /// pixi.toml 是软链接（由 dotfiles 管理）时也删除；链接目标不会被删
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct PkgArgs {
    /// 环境名，默认当前环境
    #[arg(short = 'e', long = "env", value_name = "名字")]
    pub env: Option<String>,
    /// 作为 PyPI 包（pypi-dependencies），只对 pixi 后端有意义
    #[arg(long)]
    pub pypi: bool,
    /// 包，可以带版本约束，例如 'numpy>=2'
    #[arg(required = true, value_name = "包")]
    pub packages: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    /// 环境名
    #[arg(value_name = "名字")]
    pub names: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// 环境名
    #[arg(short = 'e', long = "env", value_name = "名字", required = true)]
    pub env: String,
    /// 要运行的命令及参数，写在 -- 后面
    #[arg(last = true, required = true, value_name = "命令")]
    pub command: Vec<String>,
}

#[derive(Debug, Args)]
pub struct PathArgs {
    /// 环境名
    #[arg(value_name = "名字")]
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct WhichArgs {
    /// 环境名，默认当前环境
    #[arg(short = 'e', long = "env", value_name = "名字")]
    pub env: Option<String>,
    /// 命令名
    #[arg(value_name = "命令")]
    pub command: String,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// 不指定时按当前 shell 自动识别
    #[arg(value_enum)]
    pub shell: Option<Shell>,
}

#[derive(Debug, Args)]
pub struct HookArgs {
    #[arg(value_enum)]
    pub shell: Shell,
}

const TEMPLATE: &str = "\
{before-help}{about-with-newline}
用法: {usage}

{all-args}{after-help}";

/// clap writes `[OPTIONS]` into usage lines itself; the rest of the help text
/// is already Chinese.
pub fn localize_help(text: &str) -> String {
    text.replace("[OPTIONS]", "[选项]")
        .replace("Usage:", "用法:")
        .replace(
            "For more information, try '--help'.",
            "运行 '--help' 查看帮助。",
        )
}

/// The clap command with Chinese headings and help/version flags.
pub fn command() -> clap::Command {
    localize(Cli::command())
        .disable_help_subcommand(true)
        .disable_version_flag(true)
        .arg(
            clap::Arg::new("version")
                .short('V')
                .long("version")
                .action(ArgAction::Version)
                .help("显示版本")
                .help_heading("选项"),
        )
}

fn localize(cmd: clap::Command) -> clap::Command {
    cmd.help_template(TEMPLATE)
        .subcommand_help_heading("子命令")
        .subcommand_value_name("子命令")
        .disable_help_flag(true)
        .mut_args(|a| {
            let heading = if a.is_positional() {
                "参数"
            } else {
                "选项"
            };
            a.help_heading(heading)
        })
        .arg(
            clap::Arg::new("help")
                .short('h')
                .long("help")
                .action(ArgAction::Help)
                .help("显示帮助")
                .help_heading("选项"),
        )
        .mut_subcommands(localize)
}
