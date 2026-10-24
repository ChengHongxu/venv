//! The only place venv spawns pixi, uv, python or pip. Every child gets the
//! base environment (the current one with venv's activation undone), and its
//! stdout goes to our stderr, so nothing it prints can end up in the shell code
//! the venv function evaluates.

use std::fmt;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::EnvMap;

pub struct Cmd {
    program: PathBuf,
    args: Vec<String>,
    env: EnvMap,
}

impl Cmd {
    pub fn new(program: &Path, env: &EnvMap) -> Cmd {
        Cmd {
            program: program.to_path_buf(),
            args: Vec::new(),
            env: env.clone(),
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Cmd {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn path_arg(self, p: &Path) -> Cmd {
        self.arg(p.display().to_string())
    }

    pub fn env(mut self, k: &str, v: &str) -> Cmd {
        self.env.insert(k.into(), v.into());
        self
    }

    pub fn env_remove(mut self, k: &str) -> Cmd {
        self.env.remove(k);
        self
    }

    fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args).env_clear().envs(&self.env);
        c
    }

    fn stderr_as_stdout() -> Result<Stdio> {
        let fd = std::io::stderr().as_fd().try_clone_to_owned()?;
        Ok(Stdio::from(fd))
    }

    /// Run on the terminal; fail unless it exits 0.
    pub fn run(&self) -> Result<()> {
        let status = self
            .command()
            .stdout(Self::stderr_as_stdout()?)
            .status()
            .with_context(|| format!("无法运行 {}", self.program.display()))?;
        if !status.success() {
            bail!("命令失败（退出码 {}）: {self}", status.code().unwrap_or(-1));
        }
        Ok(())
    }

    /// Run with stdout captured and stderr on the terminal (pixi's progress).
    pub fn capture(&self) -> Result<String> {
        let out = self
            .command()
            .stdin(Stdio::inherit())
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("无法运行 {}", self.program.display()))?;
        if !out.status.success() {
            bail!(
                "命令失败（退出码 {}）: {self}",
                out.status.code().unwrap_or(-1)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run quietly; on failure return stderr so the caller can explain it.
    pub fn run_quiet(&self) -> Result<std::result::Result<(), String>> {
        let out = self
            .command()
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("无法运行 {}", self.program.display()))?;
        Ok(if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        })
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let program = self.program.display().to_string();
        let words = std::iter::once(program.as_str()).chain(self.args.iter().map(String::as_str));
        let quoted: Vec<String> = words.map(shell_word).collect();
        f.write_str(&quoted.join(" "))
    }
}

fn shell_word(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `name` on the PATH of `env`.
pub fn search_path(name: &str, env: &EnvMap) -> Option<PathBuf> {
    let path = env.get("PATH")?;
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(name))
        .find(|p| is_executable(p))
}

/// Find an executable: the configured value (a path, or a name to look up),
/// else `name` on PATH, else the usual install locations.
pub fn find(
    configured: Option<&str>,
    name: &str,
    fallbacks: &[PathBuf],
    env: &EnvMap,
) -> Option<PathBuf> {
    if let Some(c) = configured {
        return if c.contains('/') {
            Some(PathBuf::from(c)).filter(|p| is_executable(p))
        } else {
            search_path(c, env)
        };
    }
    search_path(name, env).or_else(|| fallbacks.iter().find(|p| is_executable(p)).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_quotes_only_when_needed() {
        let c = Cmd::new(Path::new("/usr/bin/pixi"), &EnvMap::new()).args([
            "add",
            "--manifest-path",
            "/a b/pixi.toml",
            "it's",
        ]);
        assert_eq!(
            c.to_string(),
            "/usr/bin/pixi add --manifest-path '/a b/pixi.toml' 'it'\\''s'"
        );
    }

    #[test]
    fn find_prefers_config_then_path_then_fallbacks() {
        let t = tempfile::tempdir().unwrap();
        let mk = |rel: &str| {
            let p = t.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        let on_path = mk("bin/tool");
        let fallback = mk("fallback/tool");
        let configured = mk("opt/tool2");
        let env: EnvMap = [(
            "PATH".into(),
            format!("/nope:{}", t.path().join("bin").display()),
        )]
        .into();

        let fallbacks = std::slice::from_ref(&fallback);
        assert_eq!(find(None, "tool", fallbacks, &env), Some(on_path));
        let c = configured.display().to_string();
        assert_eq!(find(Some(&c), "tool", &[], &env), Some(configured));
        assert_eq!(find(None, "other", &[], &env), None);
        let empty: EnvMap = EnvMap::new();
        assert_eq!(
            find(None, "tool", fallbacks, &empty),
            Some(fallback.clone())
        );
        assert_eq!(find(Some("/missing/tool"), "tool", &[], &env), None);
    }
}
