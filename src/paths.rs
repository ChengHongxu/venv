//! Every filesystem location venv reads or writes. Built once from the process
//! environment; tests point HOME and the XDG variables at a temp dir.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::EnvMap;

#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME/venv/config.toml`, default `~/.config/venv/config.toml`
    pub config_file: PathBuf,
    /// `$XDG_CACHE_HOME/venv`, default `~/.cache/venv`
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn from_env(env: &EnvMap) -> Result<Paths> {
        let home = env
            .get("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .context("HOME 没有设置")?;
        let xdg = |var: &str, default: &str| {
            env.get(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        Ok(Paths {
            config_file: xdg("XDG_CONFIG_HOME", ".config").join("venv/config.toml"),
            cache_dir: xdg("XDG_CACHE_HOME", ".cache").join("venv"),
            home,
        })
    }

    /// Cached pixi activation for one environment.
    pub fn pixi_cache(&self, name: &str) -> PathBuf {
        self.cache_dir.join("pixi").join(format!("{name}.json"))
    }

    /// Expand a leading `~/` written in the config file.
    pub fn expand(&self, s: &str) -> PathBuf {
        match s.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None if s == "~" => self.home.clone(),
            None => PathBuf::from(s),
        }
    }

    /// `/home/u/.venvs/x` -> `~/.venvs/x`, for messages only.
    pub fn tilde(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn xdg_dirs_override_defaults_only_when_absolute() {
        let p = Paths::from_env(&env(&[("HOME", "/h")])).unwrap();
        assert_eq!(p.config_file, PathBuf::from("/h/.config/venv/config.toml"));
        assert_eq!(p.cache_dir, PathBuf::from("/h/.cache/venv"));

        let p = Paths::from_env(&env(&[
            ("HOME", "/h"),
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_CACHE_HOME", "relative"),
        ]))
        .unwrap();
        assert_eq!(p.config_file, PathBuf::from("/c/venv/config.toml"));
        assert_eq!(p.cache_dir, PathBuf::from("/h/.cache/venv"));
    }

    #[test]
    fn tilde_and_expand() {
        let p = Paths::from_env(&env(&[("HOME", "/h")])).unwrap();
        assert_eq!(p.expand("~/x"), PathBuf::from("/h/x"));
        assert_eq!(p.expand("/abs"), PathBuf::from("/abs"));
        assert_eq!(p.tilde(Path::new("/h/.venvs/a")), "~/.venvs/a");
        assert_eq!(p.tilde(Path::new("/other")), "/other");
    }
}
