//! Environments on disk: `$VENVS_HOME/<name>/`, backend detected from the
//! declaration file. Nothing here runs a subprocess, so `list` stays fast.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::config::Backend;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `manifest` is `<dir>/pixi.toml` as found; it may be a symlink.
    Pixi {
        manifest: PathBuf,
    },
    Uv,
    Venv,
    /// `pixi.toml` is a symlink whose target is gone.
    Broken {
        link: PathBuf,
    },
}

impl Kind {
    pub fn backend(&self) -> Option<Backend> {
        match self {
            Kind::Pixi { .. } => Some(Backend::Pixi),
            Kind::Uv => Some(Backend::Uv),
            Kind::Venv => Some(Backend::Venv),
            Kind::Broken { .. } => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Kind::Pixi { .. } | Kind::Broken { .. } => "pixi",
            Kind::Uv => "uv",
            Kind::Venv => "venv",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Env {
    pub name: String,
    pub dir: PathBuf,
    pub kind: Kind,
}

impl Env {
    /// The real `pixi.toml` (symlinks resolved). pixi puts `.pixi/` and
    /// `pixi.lock` next to it.
    pub fn manifest(&self) -> Option<PathBuf> {
        match &self.kind {
            Kind::Pixi { manifest } => manifest.canonicalize().ok(),
            _ => None,
        }
    }

    /// Whether `pixi.toml` is a symlink (managed elsewhere, e.g. in dotfiles).
    pub fn linked_manifest(&self) -> Option<PathBuf> {
        let link = match &self.kind {
            Kind::Pixi { manifest } => manifest,
            Kind::Broken { link } => link,
            _ => return None,
        };
        link.symlink_metadata()
            .ok()
            .filter(|m| m.file_type().is_symlink())
            .and_then(|_| std::fs::read_link(link).ok())
    }

    /// Where the environment lives, without asking pixi. For pixi this is the
    /// default location; `cached_prefix` overrides it when pixi said otherwise
    /// (detached-environments).
    pub fn default_prefix(&self) -> PathBuf {
        match &self.kind {
            Kind::Pixi { .. } => self
                .manifest()
                .and_then(|m| m.parent().map(Path::to_path_buf))
                .unwrap_or_else(|| self.dir.clone())
                .join(".pixi/envs/default"),
            _ => self.dir.join(".venv"),
        }
    }
}

/// `[A-Za-z0-9._-]+`, not starting with `.` or `-`. Keeps names safe as path
/// components and as completion words.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !ok {
        bail!("环境名 '{name}' 不合法：只能用字母、数字、`.`、`_`、`-`，且不能以 `.` 或 `-` 开头");
    }
    Ok(())
}

fn present(p: &Path) -> bool {
    p.symlink_metadata().is_ok()
}

/// Backend of `dir` by which declaration file it has. A dangling `pixi.toml`
/// symlink still counts as pixi (reported as broken).
pub fn detect(dir: &Path) -> Option<Kind> {
    let manifest = dir.join("pixi.toml");
    if present(&manifest) {
        return Some(if manifest.exists() {
            Kind::Pixi { manifest }
        } else {
            Kind::Broken { link: manifest }
        });
    }
    if dir.join("pyproject.toml").exists() {
        return Some(Kind::Uv);
    }
    if dir.join("requirements.txt").exists() {
        return Some(Kind::Venv);
    }
    None
}

pub fn open(home: &Path, name: &str) -> Option<Env> {
    if validate_name(name).is_err() {
        return None;
    }
    let dir = home.join(name);
    detect(&dir).map(|kind| Env {
        name: name.to_string(),
        dir,
        kind,
    })
}

/// All environments, sorted by name. Directories without a declaration file
/// (including bare `python -m venv` dirs) are skipped.
pub fn list(home: &Path) -> Vec<Env> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut envs: Vec<Env> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| open(home, &name))
        .collect();
    envs.sort_by(|a, b| a.name.cmp(&b.name));
    envs
}

/// Python version of an installed environment, read from files only.
pub fn python_version(kind: &Kind, prefix: &Path) -> Option<String> {
    match kind {
        Kind::Pixi { .. } => conda_python(prefix),
        Kind::Uv | Kind::Venv => pyvenv_version(prefix),
        Kind::Broken { .. } => None,
    }
}

/// `conda-meta/python-3.12.5-h1234_0_cpython.json` -> `3.12.5`. Splits from
/// the right so `python-dateutil-...` is not mistaken for python.
fn conda_python(prefix: &Path) -> Option<String> {
    std::fs::read_dir(prefix.join("conda-meta"))
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .find_map(|file| {
            let stem = file.strip_suffix(".json")?;
            let mut parts = stem.rsplitn(3, '-');
            let (_build, version, name) = (parts.next()?, parts.next()?, parts.next()?);
            (name == "python" && version.starts_with(|c: char| c.is_ascii_digit()))
                .then(|| version.to_string())
        })
}

/// `version = 3.12.1` (python -m venv) or `version_info = 3.12.1` (uv).
fn pyvenv_version(prefix: &Path) -> Option<String> {
    let text = std::fs::read_to_string(prefix.join("pyvenv.cfg")).ok()?;
    let get = |key: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| v.trim().to_string())
        })
    };
    get("version").or_else(|| get("version_info"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn names() {
        for ok in ["default", "py3.12", "a_b-c", "A1"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".hidden", "-x", "a/b", "../x", "a b", "名字", "a'b"] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn detection_order_and_symlinks() {
        let t = tempfile::tempdir().unwrap();
        let mk = |name: &str, files: &[&str]| {
            let d = t.path().join(name);
            fs::create_dir_all(&d).unwrap();
            for f in files {
                fs::write(d.join(f), "").unwrap();
            }
            d
        };
        let pixi = mk("p", &["pixi.toml", "pyproject.toml"]);
        assert!(matches!(detect(&pixi), Some(Kind::Pixi { .. })));
        assert_eq!(
            detect(&mk("u", &["pyproject.toml", "requirements.txt"])),
            Some(Kind::Uv)
        );
        assert_eq!(detect(&mk("v", &["requirements.txt"])), Some(Kind::Venv));
        // A bare `python -m venv` directory is not an environment we manage
        assert_eq!(detect(&mk("bare", &["pyvenv.cfg"])), None);
        assert_eq!(detect(&mk("empty", &[])), None);

        // pixi.toml symlinked from a dotfiles repo
        let repo = mk("repo", &["pixi.toml"]);
        let linked = mk("linked", &[]);
        std::os::unix::fs::symlink(repo.join("pixi.toml"), linked.join("pixi.toml")).unwrap();
        let env = open(t.path(), "linked").unwrap();
        assert!(matches!(env.kind, Kind::Pixi { .. }));
        assert_eq!(
            env.manifest().unwrap(),
            repo.join("pixi.toml").canonicalize().unwrap()
        );
        assert_eq!(env.linked_manifest().unwrap(), repo.join("pixi.toml"));
        assert_eq!(
            env.default_prefix(),
            repo.canonicalize().unwrap().join(".pixi/envs/default")
        );

        // Dangling symlink
        let broken = mk("broken", &[]);
        std::os::unix::fs::symlink(t.path().join("gone/pixi.toml"), broken.join("pixi.toml"))
            .unwrap();
        assert!(matches!(detect(&broken), Some(Kind::Broken { .. })));

        let names: Vec<_> = list(t.path()).into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["broken", "linked", "p", "repo", "u", "v"]);
    }

    #[test]
    fn python_versions_from_files() {
        let t = tempfile::tempdir().unwrap();
        let meta = t.path().join("conda-meta");
        fs::create_dir_all(&meta).unwrap();
        for f in [
            "python-dateutil-2.9.0-pyhd8ed1ab_1.json",
            "python_abi-3.12-5_cp312.json",
            "python-3.12.15-h5f976f7_2_cpython.json",
        ] {
            fs::write(meta.join(f), "{}").unwrap();
        }
        let pixi = Kind::Pixi {
            manifest: PathBuf::new(),
        };
        assert_eq!(python_version(&pixi, t.path()).as_deref(), Some("3.12.15"));

        fs::write(
            t.path().join("pyvenv.cfg"),
            "home = /usr/bin\nversion_info = 3.13.13\n",
        )
        .unwrap();
        assert_eq!(
            python_version(&Kind::Uv, t.path()).as_deref(),
            Some("3.13.13")
        );
        fs::write(
            t.path().join("pyvenv.cfg"),
            "home = /usr/bin\nversion = 3.12.1\n",
        )
        .unwrap();
        assert_eq!(
            python_version(&Kind::Venv, t.path()).as_deref(),
            Some("3.12.1")
        );
        assert_eq!(python_version(&Kind::Venv, &t.path().join("nope")), None);
    }
}
