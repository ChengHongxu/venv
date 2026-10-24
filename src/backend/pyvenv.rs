//! python-venv: `python -m venv` + pip. The declaration is `requirements.txt`
//! (direct dependencies only, edited line by line) plus `.python-version`, so
//! `sync` can rebuild `.venv/` with the same interpreter version.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::envs::Env;
use crate::runner::{self, Cmd};
use crate::{Ctx, EnvMap, requirements, ui};

const REQUIREMENTS_HEADER: &str = "# 由 venv add / venv remove 维护；也可以手动编辑\n";

/// The interpreter to build `.venv/` with. `explicit`: the user passed
/// `--python`, so the configured interpreter does not apply and plain
/// `python3` is not an acceptable stand-in.
fn find_python(cx: &Ctx, base: &EnvMap, version: Option<&str>, explicit: bool) -> Result<PathBuf> {
    if !explicit && let Some(configured) = &cx.config.venv_python {
        return runner::find(Some(configured), "python3", &[], base)
            .ok_or_else(|| anyhow::anyhow!("配置的 venv.python（{configured}）不存在或不可执行"));
    }
    if let Some(v) = version
        && let Some(p) = runner::search_path(&format!("python{v}"), base)
    {
        return Ok(p);
    }
    if !explicit && let Some(p) = runner::search_path("python3", base) {
        return Ok(p);
    }
    let v = version.unwrap_or("3");
    bail!(
        "找不到 python{v}。可以：\n\
         \n    uv python install {v}            # 然后在配置里写 venv.python = \"<uv python find {v} 的输出>\"\
         \n    pixi global install python={v}\
         \n    用系统包管理器安装 python{v}\
         \n\n或者改用 uv 后端：venv new <名字> --backend uv --python {v}"
    )
}

fn make_venv(
    cx: &Ctx,
    dir: &Path,
    version: Option<&str>,
    explicit: bool,
    base: &EnvMap,
) -> Result<()> {
    let python = find_python(cx, base, version, explicit)?;
    let venv_dir = dir.join(".venv");
    let made = Cmd::new(&python, base)
        .args(["-m", "venv"])
        .path_arg(&venv_dir)
        .run_quiet()?;
    if let Err(stderr) = made {
        let _ = std::fs::remove_dir_all(&venv_dir);
        let hint = if stderr.contains("ensurepip") {
            "\n\n这个 python 没有 ensurepip。Debian/Ubuntu 上装 python3-venv（或 python3.X-venv），\
             或者在配置里用 venv.python 指定别的解释器，或者改用 --backend uv"
        } else {
            ""
        };
        bail!("{} -m venv 失败：\n{stderr}{hint}", python.display());
    }
    Ok(())
}

pub fn create(cx: &Ctx, dir: &Path, python: &str, explicit: bool, base: &EnvMap) -> Result<()> {
    make_venv(cx, dir, Some(python), explicit, base)?;
    // Record the version actually used, for `sync` on another machine
    let prefix = dir.join(".venv");
    if let Some(full) = crate::envs::python_version(&crate::envs::Kind::Venv, &prefix) {
        let minor: Vec<&str> = full.split('.').take(2).collect();
        std::fs::write(
            dir.join(".python-version"),
            format!("{}\n", minor.join(".")),
        )?;
        if explicit && minor.join(".") != python && !full.starts_with(&format!("{python}.")) {
            ui::warn(format!("要求的是 Python {python}，实际用的是 {full}"));
        }
    }
    std::fs::write(dir.join("requirements.txt"), REQUIREMENTS_HEADER)?;
    Ok(())
}

fn python_in(env: &Env) -> PathBuf {
    env.default_prefix().join("bin/python")
}

fn pip(env: &Env, base: &EnvMap) -> Cmd {
    Cmd::new(&python_in(env), base)
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .args(["-m", "pip"])
}

pub fn sync(cx: &Ctx, env: &Env, base: &EnvMap) -> Result<()> {
    if !python_in(env).exists() {
        let pinned = std::fs::read_to_string(env.dir.join(".python-version")).ok();
        let version = pinned.as_deref().map(str::trim).filter(|v| !v.is_empty());
        make_venv(cx, &env.dir, version, version.is_some(), base)?;
    }
    let reqs = env.dir.join("requirements.txt");
    if requirements::has_requirements(&std::fs::read_to_string(&reqs).unwrap_or_default()) {
        pip(env, base)
            .args(["install", "-r"])
            .path_arg(&reqs)
            .run()?;
    }
    Ok(())
}

pub fn add(cx: &Ctx, env: &Env, base: &EnvMap, specs: &[String]) -> Result<()> {
    if !python_in(env).exists() {
        sync(cx, env, base)?;
    }
    pip(env, base)
        .arg("install")
        .args(specs.iter().cloned())
        .run()?;
    let file = env.dir.join("requirements.txt");
    let content = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(&file, requirements::add(&content, specs))?;
    Ok(())
}

pub fn remove(_cx: &Ctx, env: &Env, base: &EnvMap, names: &[String]) -> Result<()> {
    if python_in(env).exists() {
        pip(env, base)
            .args(["uninstall", "-y"])
            .args(names.iter().cloned())
            .run()?;
    }
    let file = env.dir.join("requirements.txt");
    let content = std::fs::read_to_string(&file).unwrap_or_default();
    let (updated, missing) = requirements::remove(&content, names);
    std::fs::write(&file, updated)?;
    if !missing.is_empty() {
        ui::warn(format!("requirements.txt 里没有 {}", missing.join("、")));
    }
    Ok(())
}
