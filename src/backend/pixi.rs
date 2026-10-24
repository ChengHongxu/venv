//! pixi workspaces: `pixi.toml` + `pixi.lock`, environment in `.pixi/`.
//! `pixi.toml` may be a symlink into a dotfiles repo; every command gets the
//! real path, so `.pixi/` and `pixi.lock` land next to the real file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::activation::{Activation, classify};
use crate::envs::Env;
use crate::runner::{self, Cmd};
use crate::{Ctx, EnvMap, cache};

const INSTALL_HINT: &str = "找不到 pixi。安装方法：\n\n    curl -fsSL https://pixi.sh/install.sh | sh\n\n或在 ~/.config/venv/config.toml 的 [pixi] 里用 exe 指定路径";

pub fn exe(cx: &Ctx, base: &EnvMap) -> Result<PathBuf> {
    runner::find(
        cx.config.pixi_exe.as_deref(),
        "pixi",
        &[cx.paths.home.join(".pixi/bin/pixi")],
        base,
    )
    .context(INSTALL_HINT)
}

fn manifest(env: &Env) -> Result<PathBuf> {
    env.manifest()
        .with_context(|| format!("找不到 {} 的 pixi.toml", env.name))
}

/// `pixi init`, then add python from conda-forge.
pub fn create(cx: &Ctx, dir: &Path, python: &str, base: &EnvMap) -> Result<()> {
    let pixi = exe(cx, base)?;
    let mut init = Cmd::new(&pixi, base).arg("init").path_arg(dir);
    for channel in &cx.config.pixi_channels {
        init = init.args(["--channel", channel]);
    }
    init.run()?;
    Cmd::new(&pixi, base)
        .args(["add", "--manifest-path"])
        .path_arg(&dir.join("pixi.toml"))
        .arg(format!("python={python}"))
        .run()
}

pub fn edit(
    cx: &Ctx,
    env: &Env,
    base: &EnvMap,
    verb: &str,
    pkgs: &[String],
    pypi: bool,
) -> Result<()> {
    let pixi = exe(cx, base)?;
    let mut cmd = Cmd::new(&pixi, base)
        .args([verb, "--manifest-path"])
        .path_arg(&manifest(env)?);
    if pypi {
        cmd = cmd.arg("--pypi");
    }
    cmd.args(pkgs.iter().cloned()).run()?;
    refresh(cx, env, base);
    Ok(())
}

pub fn sync(cx: &Ctx, env: &Env, base: &EnvMap) -> Result<()> {
    let pixi = exe(cx, base)?;
    Cmd::new(&pixi, base)
        .args(["install", "--manifest-path"])
        .path_arg(&manifest(env)?)
        .run()?;
    refresh(cx, env, base);
    Ok(())
}

/// Regenerate the cache after the environment changed, so the next shell
/// starts fast. Failure only costs speed.
pub fn refresh(cx: &Ctx, env: &Env, base: &EnvMap) {
    let _ = compute(cx, env, base);
}

/// Whether activating `env` will have to run pixi (for a "please wait" note).
pub fn cache_hit(cx: &Ctx, env: &Env, base: &EnvMap) -> bool {
    lookup(cx, env, base).is_some()
}

fn lookup(cx: &Ctx, env: &Env, base: &EnvMap) -> Option<Activation> {
    let manifest = env.manifest()?;
    let pixi = exe(cx, base).ok()?;
    let key = cache::key(&manifest, &pixi)?;
    cache::lookup(&cx.paths.pixi_cache(&env.name), &key, base)
}

pub fn activation(cx: &Ctx, env: &Env, base: &EnvMap) -> Result<Activation> {
    match lookup(cx, env, base) {
        Some(act) => Ok(act),
        None => compute(cx, env, base),
    }
}

/// Ask pixi for the activation variables (installing the environment if
/// needed) and cache them relative to `base`.
fn compute(cx: &Ctx, env: &Env, base: &EnvMap) -> Result<Activation> {
    let pixi = exe(cx, base)?;
    let manifest = manifest(env)?;
    let out = Cmd::new(&pixi, base)
        .args(["shell-hook", "--json", "--manifest-path"])
        .path_arg(&manifest)
        .capture()
        .with_context(|| format!("激活 {} 失败", env.name))?;
    let json: serde_json::Value =
        serde_json::from_str(&out).context("pixi shell-hook --json 的输出无法解析")?;
    let vars: BTreeMap<String, String> = json
        .get("environment_variables")
        .and_then(|v| v.as_object())
        .context("pixi shell-hook --json 的输出里没有 environment_variables")?
        .iter()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect();
    let act = classify(base, &vars, &env.default_prefix());
    // pixi may have just written pixi.lock, so key on the files as they are now
    if let Some(key) = cache::key(&manifest, &pixi) {
        let inputs = vars
            .keys()
            .map(|k| (k.clone(), base.get(k).cloned()))
            .collect();
        cache::store(&cx.paths.pixi_cache(&env.name), &key, &act, inputs);
    }
    Ok(act)
}
