//! uv projects: `pyproject.toml` (`[tool.uv] package = false`) + `uv.lock`,
//! environment in `.venv/`. Python comes from uv's managed builds.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::envs::Env;
use crate::runner::{self, Cmd};
use crate::{Ctx, EnvMap};

const INSTALL_HINT: &str = "找不到 uv。安装方法：\n\n    curl -LsSf https://astral.sh/uv/install.sh | sh\n\n或在 ~/.config/venv/config.toml 的 [uv] 里用 exe 指定路径";

pub fn exe(cx: &Ctx, base: &EnvMap) -> Result<PathBuf> {
    runner::find(
        cx.config.uv_exe.as_deref(),
        "uv",
        &[
            cx.paths.home.join(".local/bin/uv"),
            cx.paths.home.join(".cargo/bin/uv"),
        ],
        base,
    )
    .context(INSTALL_HINT)
}

/// uv run in `dir`, without another project's VIRTUAL_ENV or a redirected
/// project environment getting in the way.
fn uv(cx: &Ctx, base: &EnvMap, dir: &Path) -> Result<Cmd> {
    Ok(Cmd::new(&exe(cx, base)?, base)
        .env_remove("VIRTUAL_ENV")
        .env_remove("UV_PROJECT_ENVIRONMENT")
        .arg("--directory")
        .path_arg(dir))
}

/// PEP 508 project name for uv: must start and end with a letter or digit.
fn project_name(name: &str) -> String {
    let trimmed = name.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        "env".into()
    } else {
        trimmed.into()
    }
}

pub fn create(cx: &Ctx, dir: &Path, name: &str, python: &str, base: &EnvMap) -> Result<()> {
    Cmd::new(&exe(cx, base)?, base)
        .env_remove("VIRTUAL_ENV")
        .args([
            "init",
            "--bare",
            "--no-workspace",
            "--pin-python",
            "--vcs",
            "none",
        ])
        .args(["--name", &project_name(name), "--python", python])
        .path_arg(dir)
        .run()?;
    let pyproject = dir.join("pyproject.toml");
    let mut text = std::fs::read_to_string(&pyproject)
        .with_context(|| format!("uv init 没有生成 {}", pyproject.display()))?;
    if !text.contains("[tool.uv]") {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        // A plain environment, not a package; python from uv, never the system
        text.push_str("\n[tool.uv]\npackage = false\npython-preference = \"only-managed\"\n");
        std::fs::write(&pyproject, text)?;
    }
    uv(cx, base, dir)?.arg("sync").run()
}

pub fn sync(cx: &Ctx, env: &Env, base: &EnvMap) -> Result<()> {
    uv(cx, base, &env.dir)?.arg("sync").run()
}

pub fn edit(cx: &Ctx, env: &Env, base: &EnvMap, verb: &str, pkgs: &[String]) -> Result<()> {
    uv(cx, base, &env.dir)?
        .arg(verb)
        .args(pkgs.iter().cloned())
        .run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn project_name_is_pep508() {
        assert_eq!(super::project_name("ml"), "ml");
        assert_eq!(super::project_name("a.b-"), "a.b");
        assert_eq!(super::project_name("_x_"), "x");
        assert_eq!(super::project_name("__"), "env");
    }
}
