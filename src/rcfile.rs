//! `venv init`: add the hook line to the shell's startup file, the way setproxy
//! does. Only the file's content changes; a symlinked startup file is written
//! through, so the link stays a link.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::EnvMap;
use crate::paths::Paths;
use crate::shell::Shell;
use crate::ui;

/// Comment written above the hook line
const COMMENT: &str = "# venv";

pub fn hook_line(shell: Shell) -> String {
    format!(r#"eval "$(venv hook {})""#, shell.name())
}

pub fn startup_file(shell: Shell, paths: &Paths, env: &EnvMap) -> PathBuf {
    match shell {
        Shell::Zsh => env
            .get("ZDOTDIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.home.clone())
            .join(".zshrc"),
        Shell::Bash => paths.home.join(".bashrc"),
    }
}

pub fn resolve(shell: Option<Shell>, env: &EnvMap) -> Result<Shell> {
    shell
        .or_else(|| detect(env))
        .context("无法识别当前 shell，请指定: venv init <zsh|bash>")
}

pub fn install(shell: Shell, paths: &Paths, env: &EnvMap) -> Result<()> {
    let file = startup_file(shell, paths, env);
    let line = hook_line(shell);
    let content = read(&file).with_context(|| format!("无法读取 {}", file.display()))?;
    let shown = describe(&file, paths);
    if has_line(&content, &line) {
        ui::ok(format!("{shown} 已配置"));
    } else {
        let (updated, replaced) = match replace_previous(&content, &line) {
            Some(updated) => (updated, true),
            None => (append(&content, &line), false),
        };
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        // fs::write follows a symlink and truncates its target in place
        fs::write(&file, updated).with_context(|| format!("无法写入 {}", file.display()))?;
        ui::ok(format!(
            "{} {shown}",
            if replaced { "已更新" } else { "已写入" }
        ));
    }
    ui::hint(format!("重开终端，或运行: source {}", paths.tilde(&file)));
    Ok(())
}

/// Shown when switch/off/new run without the shell function loaded.
pub fn not_loaded_hint(paths: &Paths, env: &EnvMap) {
    let configured = [Shell::Zsh, Shell::Bash].into_iter().find_map(|shell| {
        let file = startup_file(shell, paths, env);
        has_line(&read(&file).ok()?, &hook_line(shell)).then_some(file)
    });
    match configured {
        Some(file) => ui::fail(format!(
            "当前终端还没有加载 venv（{} 已配置）。\n\n重开终端，或运行:\n\n    source {}",
            paths.tilde(&file),
            paths.tilde(&file)
        )),
        None => ui::fail(
            "venv 还没有接入当前 shell，无法修改这个终端的环境变量。\n\n先运行一次:\n\n    venv init\n\n然后重开终端。",
        ),
    }
}

/// `~/.zshrc`, plus the real file when it is a symlink.
fn describe(file: &Path, paths: &Paths) -> String {
    match fs::read_link(file) {
        Ok(target) => {
            let target = file.parent().map(|d| d.join(&target)).unwrap_or(target);
            format!("{}（→ {}）", paths.tilde(file), paths.tilde(&target))
        }
        Err(_) => paths.tilde(file),
    }
}

/// Replace the hook line an earlier `venv init` left under "# venv", so
/// re-running init after the line changes updates it in place
fn replace_previous(content: &str, line: &str) -> Option<String> {
    let mut lines: Vec<&str> = content.lines().collect();
    let i = lines.windows(2).position(|w| {
        let next = w[1].trim();
        w[0].trim() == COMMENT && next.starts_with("eval ") && next.contains("venv ")
    })?;
    lines[i + 1] = line;
    let mut out = lines.join("\n");
    out.push('\n');
    Some(out)
}

/// Append the hook line, separated from existing content by a blank line
fn append(content: &str, line: &str) -> String {
    let mut out = content.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(&format!("{COMMENT}\n{line}\n"));
    out
}

/// Read a startup file; a missing one is empty
fn read(file: &Path) -> std::io::Result<String> {
    match fs::read_to_string(file) {
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

fn has_line(content: &str, line: &str) -> bool {
    content.lines().any(|l| l.trim() == line)
}

/// The shell `venv` was started from. Walks up the process tree, since a pixi
/// trampoline may sit in between, then falls back to $SHELL (the login shell).
fn detect(env: &EnvMap) -> Option<Shell> {
    ancestor_shell().or_else(|| env.get("SHELL").and_then(|s| from_program(s)))
}

fn from_program(path: &str) -> Option<Shell> {
    match path.rsplit('/').next()?.trim_start_matches('-') {
        "zsh" => Some(Shell::Zsh),
        "bash" => Some(Shell::Bash),
        _ => None,
    }
}

fn ancestor_shell() -> Option<Shell> {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..8 {
        if pid <= 1 {
            break;
        }
        let (name, ppid) = process_info(pid)?;
        if let Some(shell) = from_program(&name) {
            return Some(shell);
        }
        pid = ppid;
    }
    None
}

/// (name, parent pid) of a process
#[cfg(target_os = "linux")]
fn process_info(pid: u32) -> Option<(String, u32)> {
    // "pid (comm) state ppid ...", where comm may contain spaces or parentheses
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (open, close) = (stat.find('(')?, stat.rfind(')')?);
    let ppid = stat[close + 1..].split_whitespace().nth(1)?.parse().ok()?;
    Some((stat[open + 1..close].to_string(), ppid))
}

/// (name, parent pid) of a process
#[cfg(not(target_os = "linux"))]
fn process_info(pid: u32) -> Option<(String, u32)> {
    let output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let output = String::from_utf8_lossy(&output.stdout);
    let (ppid, name) = output.trim().split_once(char::is_whitespace)?;
    Some((name.trim().to_string(), ppid.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_names() {
        assert_eq!(from_program("/usr/bin/zsh"), Some(Shell::Zsh));
        assert_eq!(from_program("-bash"), Some(Shell::Bash));
        assert_eq!(from_program("/bin/sh"), None);
    }

    #[test]
    fn replace_and_append() {
        let line = hook_line(Shell::Zsh);
        let rc = "export A=1\n# venv\neval \"$(venv init zsh)\"\nexport B=2\n";
        assert_eq!(
            replace_previous(rc, &line).unwrap(),
            format!("export A=1\n# venv\n{line}\nexport B=2\n")
        );
        // The user's own lines under a similar comment stay
        assert_eq!(replace_previous("# venv\nalias v=venv\n", &line), None);

        assert_eq!(append("", &line), format!("# venv\n{line}\n"));
        assert_eq!(
            append("export A=1", &line),
            format!("export A=1\n\n# venv\n{line}\n")
        );
    }
}
