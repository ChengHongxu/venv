//! The three backends. Each one only runs the backend's own commands; venv
//! computes activation itself (no `bin/activate`, no pixi() wrapper function).

pub mod pixi;
pub mod pyvenv;
pub mod uv;

use anyhow::{Result, bail};

use crate::Ctx;
use crate::activation::Activation;
use crate::envs::{Env, Kind};
use crate::ui;

/// What activating `env` changes. Builds a missing uv/python-venv environment
/// from its declaration file first; pixi installs on its own.
pub fn activation(cx: &Ctx, env: &Env, base: &crate::EnvMap) -> Result<Activation> {
    match &env.kind {
        Kind::Pixi { .. } => pixi::activation(cx, env, base),
        Kind::Uv | Kind::Venv => {
            let prefix = env.default_prefix();
            if !prefix.join("bin/python").exists() {
                ui::info(format!("{} 还没有安装，先按声明文件创建", env.name));
                sync(cx, env, base)?;
            }
            Ok(Activation::virtualenv(&prefix))
        }
        Kind::Broken { link } => broken(link),
    }
}

/// Install or update `env` from its declaration file.
pub fn sync(cx: &Ctx, env: &Env, base: &crate::EnvMap) -> Result<()> {
    match &env.kind {
        Kind::Pixi { .. } => pixi::sync(cx, env, base),
        Kind::Uv => uv::sync(cx, env, base),
        Kind::Venv => pyvenv::sync(cx, env, base),
        Kind::Broken { link } => broken(link),
    }
}

pub fn add(cx: &Ctx, env: &Env, base: &crate::EnvMap, pkgs: &[String], pypi: bool) -> Result<()> {
    match &env.kind {
        Kind::Pixi { .. } => pixi::edit(cx, env, base, "add", pkgs, pypi),
        Kind::Uv => uv::edit(cx, env, base, "add", pkgs),
        Kind::Venv => pyvenv::add(cx, env, base, pkgs),
        Kind::Broken { link } => broken(link),
    }
}

pub fn remove(
    cx: &Ctx,
    env: &Env,
    base: &crate::EnvMap,
    pkgs: &[String],
    pypi: bool,
) -> Result<()> {
    match &env.kind {
        Kind::Pixi { .. } => pixi::edit(cx, env, base, "remove", pkgs, pypi),
        Kind::Uv => uv::edit(cx, env, base, "remove", pkgs),
        Kind::Venv => pyvenv::remove(cx, env, base, pkgs),
        Kind::Broken { link } => broken(link),
    }
}

fn broken<T>(link: &std::path::Path) -> Result<T> {
    let target = std::fs::read_link(link)
        .map(|t| t.display().to_string())
        .unwrap_or_default();
    bail!(
        "{} 是坏掉的软链接（指向 {target}，目标不存在）",
        link.display()
    )
}
