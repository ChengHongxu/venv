//! `~/.config/venv/config.toml`. Every key is optional; a missing file means all
//! defaults. Unknown keys are an error so a typo is not silently ignored.

use std::fmt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::EnvMap;
use crate::paths::Paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Pixi,
    Uv,
    Venv,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Backend::Pixi => "pixi",
            Backend::Uv => "uv",
            Backend::Venv => "venv",
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    home: Option<String>,
    default_backend: Option<Backend>,
    default_python: Option<String>,
    auto_activate: Option<String>,
    #[serde(default)]
    pixi: RawPixi,
    #[serde(default)]
    uv: RawUv,
    #[serde(default)]
    venv: RawVenv,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPixi {
    channels: Option<Vec<String>>,
    exe: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUv {
    exe: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVenv {
    python: Option<String>,
}

/// Where a setting's value came from, shown by `venv config`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    Env,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Default => "默认值",
            Source::File => "配置文件",
            Source::Env => "环境变量 VENVS_HOME",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub file_exists: bool,
    pub home: PathBuf,
    pub home_source: Source,
    pub default_backend: Backend,
    pub default_python: String,
    pub auto_activate: Option<String>,
    pub pixi_channels: Vec<String>,
    pub pixi_exe: Option<String>,
    pub uv_exe: Option<String>,
    pub venv_python: Option<String>,
    sources: Vec<(&'static str, Source)>,
}

pub const DEFAULT_PYTHON: &str = "3.14";

impl Config {
    pub fn load(paths: &Paths, env: &EnvMap) -> Result<Config> {
        let (raw, file_exists) = match std::fs::read_to_string(&paths.config_file) {
            Ok(text) => {
                let raw: RawConfig = toml::from_str(&text).with_context(|| {
                    format!("配置文件 {} 有误", paths.tilde(&paths.config_file))
                })?;
                (raw, true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (RawConfig::default(), false),
            Err(e) => {
                return Err(e).with_context(|| format!("无法读取 {}", paths.config_file.display()));
            }
        };
        Ok(Config::resolve(raw, file_exists, paths, env))
    }

    fn resolve(raw: RawConfig, file_exists: bool, paths: &Paths, env: &EnvMap) -> Config {
        let src = |set: bool| if set { Source::File } else { Source::Default };
        let sources = vec![
            ("default_backend", src(raw.default_backend.is_some())),
            ("default_python", src(raw.default_python.is_some())),
            ("auto_activate", src(raw.auto_activate.is_some())),
            ("pixi.channels", src(raw.pixi.channels.is_some())),
            ("pixi.exe", src(raw.pixi.exe.is_some())),
            ("uv.exe", src(raw.uv.exe.is_some())),
            ("venv.python", src(raw.venv.python.is_some())),
        ];
        let (home, home_source) = match env.get("VENVS_HOME").filter(|v| !v.is_empty()) {
            Some(v) => (paths.expand(v), Source::Env),
            None => match &raw.home {
                Some(h) => (paths.expand(h), Source::File),
                None => (paths.home.join(".venvs"), Source::Default),
            },
        };
        let nonempty = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
        Config {
            file_exists,
            home,
            home_source,
            default_backend: raw.default_backend.unwrap_or(Backend::Pixi),
            default_python: nonempty(raw.default_python).unwrap_or_else(|| DEFAULT_PYTHON.into()),
            auto_activate: nonempty(raw.auto_activate),
            pixi_channels: raw
                .pixi
                .channels
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| vec!["conda-forge".into()]),
            pixi_exe: nonempty(raw.pixi.exe),
            uv_exe: nonempty(raw.uv.exe),
            venv_python: nonempty(raw.venv.python),
            sources,
        }
    }

    pub fn source(&self, key: &str) -> Source {
        self.sources
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, s)| *s)
            .unwrap_or(Source::Default)
    }

    /// The effective configuration as TOML, each line annotated with its source.
    pub fn render(&self, paths: &Paths) -> String {
        let q = |s: &str| format!("{s:?}");
        let opt = |s: &Option<String>| s.as_deref().map(q);
        let channels = format!(
            "[{}]",
            self.pixi_channels
                .iter()
                .map(|c| q(c))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let home = q(&paths.tilde(&self.home));
        let backend = q(&self.default_backend.to_string());
        let python = q(&self.default_python);
        let lines: Vec<(Option<&str>, String, Option<String>, Source)> = vec![
            (None, "home".into(), Some(home), self.home_source),
            (
                None,
                "default_backend".into(),
                Some(backend),
                self.source("default_backend"),
            ),
            (
                None,
                "default_python".into(),
                Some(python),
                self.source("default_python"),
            ),
            (
                None,
                "auto_activate".into(),
                opt(&self.auto_activate),
                self.source("auto_activate"),
            ),
            (
                Some("pixi"),
                "channels".into(),
                Some(channels),
                self.source("pixi.channels"),
            ),
            (
                Some("pixi"),
                "exe".into(),
                opt(&self.pixi_exe),
                self.source("pixi.exe"),
            ),
            (
                Some("uv"),
                "exe".into(),
                opt(&self.uv_exe),
                self.source("uv.exe"),
            ),
            (
                Some("venv"),
                "python".into(),
                opt(&self.venv_python),
                self.source("venv.python"),
            ),
        ];
        let mut out = String::new();
        let mut section = None;
        for (sec, key, value, source) in lines {
            if sec != section {
                out.push_str(&format!("\n[{}]\n", sec.unwrap_or_default()));
                section = sec;
            }
            let line = match value {
                Some(v) => format!("{key} = {v}"),
                None => format!("# {key} = （未设置）"),
            };
            out.push_str(&format!(
                "{}  # {}\n",
                crate::ui::pad(&line, 40),
                source.label()
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(text: Option<&str>, env: &[(&str, &str)]) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        let mut map: EnvMap = [("HOME".to_string(), dir.path().display().to_string())].into();
        map.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let paths = Paths::from_env(&map).unwrap();
        if let Some(text) = text {
            std::fs::create_dir_all(paths.config_file.parent().unwrap()).unwrap();
            std::fs::write(&paths.config_file, text).unwrap();
        }
        Config::load(&paths, &map)
    }

    #[test]
    fn defaults_without_file() {
        let c = setup(None, &[]).unwrap();
        assert!(!c.file_exists);
        assert!(c.home.ends_with(".venvs"));
        assert_eq!(c.home_source, Source::Default);
        assert_eq!(c.default_backend, Backend::Pixi);
        assert_eq!(c.default_python, "3.14");
        assert_eq!(c.auto_activate, None);
        assert_eq!(c.pixi_channels, ["conda-forge"]);
    }

    #[test]
    fn file_values_and_env_precedence() {
        let text = r#"
home = "~/envs"
default_backend = "uv"
default_python = "3.12"
auto_activate = "default"
[pixi]
channels = ["https://example.com/conda", "conda-forge"]
exe = "/opt/pixi"
[venv]
python = "python3.12"
"#;
        let c = setup(Some(text), &[]).unwrap();
        assert!(c.home.ends_with("envs"));
        assert_eq!(c.home_source, Source::File);
        assert_eq!(c.default_backend, Backend::Uv);
        assert_eq!(c.auto_activate.as_deref(), Some("default"));
        assert_eq!(c.pixi_channels.len(), 2);
        assert_eq!(c.pixi_exe.as_deref(), Some("/opt/pixi"));
        assert_eq!(c.source("uv.exe"), Source::Default);

        let c = setup(Some(text), &[("VENVS_HOME", "/elsewhere")]).unwrap();
        assert_eq!(c.home, PathBuf::from("/elsewhere"));
        assert_eq!(c.home_source, Source::Env);
    }

    #[test]
    fn empty_auto_activate_means_off_and_unknown_keys_fail() {
        let c = setup(Some("auto_activate = \"\"\n"), &[]).unwrap();
        assert_eq!(c.auto_activate, None);
        let err = setup(Some("auto_activte = \"default\"\n"), &[]).unwrap_err();
        assert!(format!("{err:#}").contains("auto_activte"), "{err:#}");
        assert!(setup(Some("default_backend = \"conda\"\n"), &[]).is_err());
    }
}
