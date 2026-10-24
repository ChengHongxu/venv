//! venv: named Python environments backed by pixi, uv or python-venv, switched
//! in the current shell through a small shell function.

pub mod activation;
pub mod backend;
pub mod cache;
pub mod cli;
pub mod commands;
pub mod config;
pub mod envs;
pub mod paths;
pub mod prompt;
pub mod rcfile;
pub mod requirements;
pub mod runner;
pub mod shell;
pub mod ui;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::activation::{State, restore};
use crate::cli::{Cli, Command};
use crate::config::Config;
use crate::paths::Paths;
use crate::shell::Shell;

/// Environment variables, sorted so generated code is deterministic.
pub type EnvMap = BTreeMap<String, String>;

/// Everything a command needs from the outside world.
pub struct Ctx {
    pub paths: Paths,
    pub config: Config,
    /// The calling shell's environment (trampoline variables removed).
    pub env: EnvMap,
    /// Set when the shell function called us: print code for it to evaluate.
    pub emit: Option<Shell>,
}

/// An error whose message was already printed.
#[derive(Debug)]
pub struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("已报告")
    }
}

impl std::error::Error for Reported {}

impl Ctx {
    /// The environment with venv's activation undone. Backend commands run in
    /// it, and activation is computed from it, so the previous environment's
    /// variables never leak into the next one.
    pub fn base_env(&self) -> EnvMap {
        match State::from_env(&self.env) {
            Some(Ok(state)) => restore(&self.env, &state),
            Some(Err(_)) => {
                let mut env = self.env.clone();
                env.remove(activation::STATE_VAR);
                env
            }
            None => self.env.clone(),
        }
    }

    pub fn state(&self) -> Option<State> {
        State::from_env(&self.env).and_then(Result::ok)
    }
}

pub fn process_env() -> EnvMap {
    let env: EnvMap = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    match std::env::current_exe()
        .ok()
        .and_then(|e| e.canonicalize().ok())
    {
        Some(exe) => without_trampoline(env, &exe),
        None => env,
    }
}

/// Run directly through `~/.pixi/bin/venv` (a pixi global trampoline), venv sees
/// CONDA_PREFIX/CONDA_SHLVL set to its own install prefix and that prefix's bin
/// first on PATH. They are not the shell's, so drop them. The shell function
/// calls the real executable and never gets here.
fn without_trampoline(mut env: EnvMap, exe: &Path) -> EnvMap {
    let Some(prefix) = exe.parent().and_then(Path::parent) else {
        return env;
    };
    let ours = env
        .get("CONDA_PREFIX")
        .and_then(|p| Path::new(p).canonicalize().ok())
        .is_some_and(|p| p == prefix && prefix.join("conda-meta").is_dir());
    if !ours {
        return env;
    }
    env.remove("CONDA_PREFIX");
    env.remove("CONDA_SHLVL");
    if let Some(path) = env.get("PATH").cloned() {
        let bin = prefix.join("bin");
        if let Some((first, rest)) = path.split_once(':')
            && Path::new(first).canonicalize().ok().as_deref() == Some(bin.as_path())
        {
            env.insert("PATH".into(), rest.to_string());
        }
    }
    env
}

pub fn run(cli: Cli) -> Result<()> {
    let env = process_env();
    // hook needs no config: it must work even with a broken config file
    if let Command::Hook(args) = &cli.command {
        return commands::hook(args.shell);
    }
    let paths = Paths::from_env(&env)?;
    let config = Config::load(&paths, &env)?;
    let cx = Ctx {
        paths,
        config,
        env,
        emit: cli.emit,
    };
    commands::dispatch(&cx, cli.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trampoline_variables_are_dropped_only_for_our_prefix() {
        let t = tempfile::tempdir().unwrap();
        let prefix = t.path().join("envs/venv");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        let exe = prefix.join("bin/venv");
        std::fs::write(&exe, "").unwrap();
        let p = prefix.display().to_string();
        let env: EnvMap = [
            ("CONDA_PREFIX".into(), p.clone()),
            ("CONDA_SHLVL".into(), "1".into()),
            ("PATH".into(), format!("{p}/bin:/usr/bin")),
        ]
        .into();
        let cleaned = without_trampoline(env, &exe.canonicalize().unwrap());
        assert_eq!(cleaned.get("CONDA_PREFIX"), None);
        assert_eq!(cleaned.get("CONDA_SHLVL"), None);
        assert_eq!(cleaned["PATH"], "/usr/bin");

        // Someone else's conda env stays
        let env: EnvMap = [("CONDA_PREFIX".into(), "/opt/conda".into())].into();
        let kept = without_trampoline(env.clone(), &exe);
        assert_eq!(kept, env);
    }
}
