//! One function per subcommand. Shell code goes to stdout only in `--emit`
//! mode; data (`list`, `path`, `which`, `config`, `hook`) goes to stdout;
//! everything for humans goes to stderr.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::activation::{State, apply, diff, relocate, restore};
use crate::cli::*;
use crate::config::Backend;
use crate::envs::{self, Env, Kind};
use crate::shell::Shell;
use crate::{Ctx, EnvMap, Reported, backend, cache, prompt, rcfile, runner, ui};

pub fn dispatch(cx: &Ctx, command: Command) -> Result<()> {
    match command {
        Command::New(a) => new(cx, a),
        Command::Switch(a) => switch(cx, a.name.as_deref()),
        Command::Off => off(cx),
        Command::List(a) => list(cx, &a),
        Command::Rm(a) => rm(cx, &a),
        Command::Add(a) => packages(cx, &a, true),
        Command::Remove(a) => packages(cx, &a, false),
        Command::Sync(a) => sync(cx, &a.names),
        Command::Run(a) => run(cx, &a),
        Command::Path(a) => path(cx, a.name.as_deref()),
        Command::Which(a) => which(cx, &a),
        Command::Init(a) => {
            let shell = rcfile::resolve(a.shell, &cx.env)?;
            rcfile::install(shell, &cx.paths, &cx.env)
        }
        Command::Hook(a) => hook(a.shell),
        Command::Config => config(cx),
        Command::Startup => {
            // Must never break shell startup: warn and carry on
            if let Err(e) = startup(cx) {
                ui::warn(format!("venv: {e:#}"));
            }
            Ok(())
        }
    }
}

fn new(cx: &Ctx, a: NewArgs) -> Result<()> {
    envs::validate_name(&a.name)?;
    let dir = cx.config.home.join(&a.name);
    if dir.symlink_metadata().is_ok() {
        bail!("{} 已存在", cx.paths.tilde(&dir));
    }
    let backend = a.backend.unwrap_or(cx.config.default_backend);
    let explicit = a.python.is_some();
    let python = a.python.unwrap_or_else(|| cx.config.default_python.clone());
    let base = cx.base_env();
    // Fail on a missing tool before creating anything
    match backend {
        Backend::Pixi => {
            backend::pixi::exe(cx, &base)?;
        }
        Backend::Uv => {
            backend::uv::exe(cx, &base)?;
        }
        Backend::Venv => {}
    }
    ui::info(format!(
        "创建 {}（{backend}，Python {python}）",
        ui::bold(&a.name)
    ));
    std::fs::create_dir_all(&dir)?;
    let created = match backend {
        Backend::Pixi => backend::pixi::create(cx, &dir, &python, &base),
        Backend::Uv => backend::uv::create(cx, &dir, &a.name, &python, &base),
        Backend::Venv => backend::pyvenv::create(cx, &dir, &python, explicit, &base),
    };
    if let Err(e) = created {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    ui::ok(format!("已创建 {}", cx.paths.tilde(&dir)));
    if a.no_switch {
        // Fill the activation cache now, so the first switch is fast
        if backend == Backend::Pixi
            && let Some(env) = envs::open(&cx.config.home, &a.name)
        {
            backend::pixi::refresh(cx, &env, &base);
        }
        return Ok(());
    }
    match cx.emit {
        Some(shell) => emit_switch(cx, shell, &find(cx, &a.name)?),
        None => {
            ui::warn("没有激活：当前终端还没有加载 venv 的 shell 集成");
            ui::hint(format!(
                "加载后运行 venv switch {}，或先运行 venv init",
                a.name
            ));
            Ok(())
        }
    }
}

/// Undo the current activation, apply `env`'s, print the difference as code.
fn emit_switch(cx: &Ctx, shell: Shell, env: &Env) -> Result<()> {
    let base = cx.base_env();
    let act = backend::activation(cx, env, &base)?;
    let target = apply(&base, &env.name, &act);
    print!("{}", shell.render(&diff(&cx.env, &target)));
    ui::ok(format!("已切换到 {}", ui::bold(&env.name)));
    ui::hint(format!("路径    {}", cx.paths.tilde(&act.prefix)));
    let python = envs::python_version(&env.kind, &act.prefix).unwrap_or_else(|| "未知".into());
    ui::hint(format!("Python  {python}"));
    Ok(())
}

fn not_loaded(cx: &Ctx) -> Result<()> {
    rcfile::not_loaded_hint(&cx.paths, &cx.env);
    Err(Reported.into())
}

fn switch(cx: &Ctx, name: Option<&str>) -> Result<()> {
    let Some(name) = name else {
        print_table(cx, &mut std::io::stderr())?;
        ui::hint(ui::dim("用法: venv switch <名字>"));
        return Ok(());
    };
    let Some(shell) = cx.emit else {
        return not_loaded(cx);
    };
    emit_switch(cx, shell, &find(cx, name)?)
}

fn find(cx: &Ctx, name: &str) -> Result<Env> {
    envs::validate_name(name)?;
    if let Some(env) = envs::open(&cx.config.home, name) {
        return Ok(env);
    }
    let dir = cx.config.home.join(name);
    if dir.is_dir() {
        bail!(
            "{} 里没有 pixi.toml、pyproject.toml 或 requirements.txt，不是 venv 管理的环境",
            cx.paths.tilde(&dir)
        );
    }
    let names: Vec<String> = envs::list(&cx.config.home)
        .into_iter()
        .map(|e| e.name)
        .collect();
    let have = if names.is_empty() {
        "还没有任何环境".to_string()
    } else {
        format!("现有环境：{}", names.join("、"))
    };
    bail!("环境 '{name}' 不存在，用 venv new {name} 创建（{have}）")
}

fn off(cx: &Ctx) -> Result<()> {
    let Some(shell) = cx.emit else {
        return not_loaded(cx);
    };
    let Some(state) = cx.state() else {
        ui::warn("当前没有激活的环境");
        return Ok(());
    };
    print!("{}", shell.render(&diff(&cx.env, &cx.base_env())));
    ui::ok(format!("已退出 {}", ui::bold(&state.name)));
    Ok(())
}

/// Run by the shell integration in every new shell.
fn startup(cx: &Ctx) -> Result<()> {
    let Some(shell) = cx.emit else {
        bail!("startup 只供 shell 集成调用");
    };
    match State::from_env(&cx.env) {
        Some(Ok(state)) => {
            if state.prefix.is_dir() {
                // Nested shell / tmux pane: variables are inherited, only the
                // PATH order may have been changed by rc files or path_helper
                if let Some(env) = relocate(&cx.env, &state) {
                    print!("{}", shell.render(&diff(&cx.env, &env)));
                }
            } else {
                print!(
                    "{}",
                    shell.render(&diff(&cx.env, &restore(&cx.env, &state)))
                );
                ui::warn(format!("环境 {} 已经不存在，已退出", state.name));
            }
            Ok(())
        }
        Some(Err(e)) => {
            let mut env = cx.env.clone();
            env.remove(crate::activation::STATE_VAR);
            print!("{}", shell.render(&diff(&cx.env, &env)));
            Err(e)
        }
        None => {
            let Some(name) = &cx.config.auto_activate else {
                return Ok(());
            };
            let env = find(cx, name).map_err(|e| e.context("auto_activate"))?;
            let base = cx.base_env();
            if matches!(env.kind, Kind::Pixi { .. }) && !backend::pixi::cache_hit(cx, &env, &base) {
                ui::info(format!("正在激活 {name}…"));
            }
            let act = backend::activation(cx, &env, &base)?;
            print!(
                "{}",
                shell.render(&diff(&cx.env, &apply(&base, name, &act)))
            );
            Ok(())
        }
    }
}

/// Prefix without running anything: pixi may keep it elsewhere
/// (detached-environments), which the cache knows.
fn prefix_of(cx: &Ctx, env: &Env) -> PathBuf {
    match env.kind {
        Kind::Pixi { .. } => cache::cached_prefix(&cx.paths.pixi_cache(&env.name))
            .unwrap_or_else(|| env.default_prefix()),
        _ => env.default_prefix(),
    }
}

struct Row {
    name: String,
    backend: &'static str,
    python: Option<String>,
    current: bool,
    prefix: PathBuf,
    path: PathBuf,
    linked: Option<PathBuf>,
    broken: bool,
}

fn rows(cx: &Ctx) -> Vec<Row> {
    let current = cx.state().map(|s| s.name);
    envs::list(&cx.config.home)
        .into_iter()
        .map(|env| {
            let prefix = prefix_of(cx, &env);
            Row {
                python: envs::python_version(&env.kind, &prefix),
                current: current.as_deref() == Some(env.name.as_str()),
                backend: env.kind.label(),
                linked: env.linked_manifest(),
                broken: matches!(env.kind, Kind::Broken { .. }),
                name: env.name,
                path: env.dir,
                prefix,
            }
        })
        .collect()
}

fn print_table(cx: &Ctx, out: &mut dyn std::io::Write) -> Result<()> {
    let rows = rows(cx);
    if rows.is_empty() {
        ui::warn(format!("{} 下还没有环境", cx.paths.tilde(&cx.config.home)));
        ui::hint("用 venv new <名字> 创建一个");
        return Ok(());
    }
    let width = rows
        .iter()
        .map(|r| console::measure_text_width(&r.name))
        .max()
        .unwrap_or(0);
    for r in &rows {
        let mark = if r.current { "*" } else { " " };
        let python = if r.broken {
            "损坏".to_string()
        } else {
            r.python.clone().unwrap_or_else(|| "未安装".into())
        };
        let mut line = format!(
            "{mark} {}  {:<4}  {python}",
            ui::pad(&r.name, width),
            r.backend
        );
        if let Some(target) = &r.linked {
            line.push_str(&format!("  → {}", cx.paths.tilde(target)));
        }
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(())
}

fn list(cx: &Ctx, a: &ListArgs) -> Result<()> {
    if a.names {
        for env in envs::list(&cx.config.home) {
            println!("{}", env.name);
        }
        return Ok(());
    }
    if !a.json {
        return print_table(cx, &mut std::io::stdout());
    }
    let json: Vec<serde_json::Value> = rows(cx)
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "backend": r.backend,
                "python": r.python,
                "current": r.current,
                "installed": r.prefix.is_dir(),
                "path": r.path,
                "prefix": r.prefix,
                "manifest_link": r.linked,
                "broken": r.broken,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

fn rm(cx: &Ctx, a: &RmArgs) -> Result<()> {
    let env = find(cx, &a.name)?;
    if cx.state().is_some_and(|s| s.name == env.name) {
        bail!("{} 正在使用，先运行 venv off", env.name);
    }
    let link = env.linked_manifest();
    if let Some(target) = &link
        && !a.force
    {
        bail!(
            "{} 的 pixi.toml 是软链接，由 dotfiles 管理（→ {}）。确定要删除这个环境请加 --force；链接目标不会被删",
            env.name,
            cx.paths.tilde(target)
        );
    }
    // With a symlinked manifest the environment itself sits next to the target
    let outside = env
        .manifest()
        .and_then(|m| m.parent().map(|d| d.join(".pixi")))
        .filter(|p| !p.starts_with(&env.dir) && p.is_dir());
    let mut what = vec![cx.paths.tilde(&env.dir)];
    if let Some(p) = &outside {
        what.push(cx.paths.tilde(p));
    }
    prompt::confirm(&format!("删除 {}？", what.join(" 和 ")), a.yes)?;

    if env.dir.symlink_metadata()?.file_type().is_symlink() {
        std::fs::remove_file(&env.dir)?;
    } else {
        // Does not follow symlinks, so a linked pixi.toml's target stays
        std::fs::remove_dir_all(&env.dir)?;
    }
    if let Some(p) = &outside {
        std::fs::remove_dir_all(p)?;
    }
    let _ = std::fs::remove_file(cx.paths.pixi_cache(&env.name));
    ui::ok(format!("已删除 {}", env.name));
    if let Some(target) = &link {
        let kept = Path::new(target).with_file_name("pixi.lock");
        ui::hint(format!(
            "保留了 dotfiles 里的 {} 和 {}",
            cx.paths.tilde(target),
            cx.paths.tilde(&kept)
        ));
    }
    Ok(())
}

/// `-e NAME`, or the active environment.
fn target(cx: &Ctx, name: Option<&str>) -> Result<Env> {
    match name {
        Some(n) => find(cx, n),
        None => match cx.state() {
            Some(state) => find(cx, &state.name),
            None => bail!("当前没有激活的环境；先 venv switch <名字>，或用 -e <名字> 指定"),
        },
    }
}

fn packages(cx: &Ctx, a: &PkgArgs, add: bool) -> Result<()> {
    let env = target(cx, a.env.as_deref())?;
    if a.pypi && !matches!(env.kind, Kind::Pixi { .. }) {
        bail!(
            "--pypi 只用于 pixi 环境；{} 是 {} 环境，包本来就来自 PyPI",
            env.name,
            env.kind.label()
        );
    }
    let base = cx.base_env();
    if add {
        backend::add(cx, &env, &base, &a.packages, a.pypi)?;
    } else {
        backend::remove(cx, &env, &base, &a.packages, a.pypi)?;
    }
    let pkgs = a.packages.join(" ");
    if add {
        ui::ok(format!("已向 {} 加入 {pkgs}", env.name));
    } else {
        ui::ok(format!("已从 {} 删除 {pkgs}", env.name));
    }
    Ok(())
}

fn sync(cx: &Ctx, names: &[String]) -> Result<()> {
    let targets = if names.is_empty() {
        envs::list(&cx.config.home)
    } else {
        names.iter().map(|n| find(cx, n)).collect::<Result<_>>()?
    };
    if targets.is_empty() {
        ui::warn(format!("{} 下还没有环境", cx.paths.tilde(&cx.config.home)));
        return Ok(());
    }
    let base = cx.base_env();
    let mut failed = Vec::new();
    for env in &targets {
        ui::info(format!("{}（{}）", ui::bold(&env.name), env.kind.label()));
        if let Err(e) = backend::sync(cx, env, &base) {
            ui::fail(format!("{}: {e:#}", env.name));
            failed.push(env.name.clone());
        }
    }
    if !failed.is_empty() {
        ui::fail(format!(
            "{} 个环境失败：{}",
            failed.len(),
            failed.join("、")
        ));
        return Err(Reported.into());
    }
    ui::ok(format!("已同步 {} 个环境", targets.len()));
    Ok(())
}

fn run(cx: &Ctx, a: &RunArgs) -> Result<()> {
    let env = find(cx, &a.env)?;
    let base = cx.base_env();
    let act = backend::activation(cx, &env, &base)?;
    let target: EnvMap = apply(&base, &env.name, &act);
    let program = &a.command[0];
    let resolved = if program.contains('/') {
        PathBuf::from(program)
    } else {
        match runner::search_path(program, &target) {
            Some(p) => p,
            None => bail!("在 {} 里找不到命令 {program}", env.name),
        }
    };
    let err = std::process::Command::new(&resolved)
        .args(&a.command[1..])
        .env_clear()
        .envs(&target)
        .exec();
    bail!("无法运行 {}: {err}", resolved.display())
}

fn path(cx: &Ctx, name: Option<&str>) -> Result<()> {
    let prefix = match name {
        Some(n) => prefix_of(cx, &find(cx, n)?),
        None => match cx.state() {
            Some(state) => state.prefix,
            None => bail!("当前没有激活的环境；用 venv path <名字> 指定"),
        },
    };
    println!("{}", prefix.display());
    Ok(())
}

fn which(cx: &Ctx, a: &WhichArgs) -> Result<()> {
    let env = target(cx, a.env.as_deref())?;
    let candidate = prefix_of(cx, &env).join("bin").join(&a.command);
    let executable = candidate.metadata().is_ok_and(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    });
    if !executable {
        bail!("{} 里没有 {}", env.name, a.command);
    }
    println!("{}", candidate.display());
    Ok(())
}

pub fn hook(shell: Shell) -> Result<()> {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "venv".into());
    print!("{}", shell.hook(&bin));
    Ok(())
}

fn config(cx: &Ctx) -> Result<()> {
    let status = if cx.config.file_exists {
        ""
    } else {
        "（不存在，全部使用默认值）"
    };
    println!(
        "# 配置文件: {}{status}",
        cx.paths.tilde(&cx.paths.config_file)
    );
    print!("{}", cx.config.render(&cx.paths));
    Ok(())
}
