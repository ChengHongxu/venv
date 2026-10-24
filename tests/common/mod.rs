//! Test harness: a throwaway HOME, VENVS_HOME and XDG dirs per test, the real
//! `venv` binary, and helpers to fake environments without network access.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub const BIN: &str = env!("CARGO_BIN_EXE_venv");

pub struct Sandbox {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub venvs: PathBuf,
    /// Put first on PATH; fake tools go here.
    pub bin: PathBuf,
    /// Extra variables for every command.
    pub extra: Vec<(String, String)>,
    /// Base PATH (after `bin`).
    pub path: String,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        Sandbox::with_venvs_dir("venvs")
    }

    /// `venvs_dir` may contain spaces or quotes, to test quoting.
    pub fn with_venvs_dir(venvs_dir: &str) -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        let venvs = root.join(venvs_dir);
        let bin = root.join("bin");
        for d in [&home, &venvs, &bin] {
            fs::create_dir_all(d).unwrap();
        }
        Sandbox {
            _dir: dir,
            root,
            home,
            venvs,
            bin,
            extra: Vec::new(),
            path: "/usr/local/bin:/usr/bin:/bin".into(),
        }
    }

    pub fn full_path(&self) -> String {
        format!("{}:{}", self.bin.display(), self.path)
    }

    fn apply_env(&self, c: &mut Command) {
        c.env_clear()
            .env("HOME", &self.home)
            .env("VENVS_HOME", &self.venvs)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            .env("PATH", self.full_path())
            .env("LANG", "C.UTF-8")
            .env("TERM", "dumb");
        for (k, v) in &self.extra {
            c.env(k, v);
        }
    }

    pub fn venv(&self, args: &[&str]) -> Output {
        let mut c = Command::new(BIN);
        c.args(args).current_dir(&self.root);
        self.apply_env(&mut c);
        c.output().unwrap()
    }

    /// Run `script` in `shell` (`zsh` or `bash`) with no rc files, after
    /// loading the integration.
    pub fn shell(&self, shell: &str, script: &str) -> Output {
        let quoted = BIN.replace('\'', r"'\''");
        let full = format!("eval \"$('{quoted}' hook {shell})\"\n{script}");
        let mut c = Command::new(shell);
        match shell {
            "zsh" => c.args(["-f", "-c", &full]),
            _ => c.args(["--norc", "--noprofile", "-c", &full]),
        };
        c.current_dir(&self.root);
        self.apply_env(&mut c);
        c.output().unwrap()
    }

    pub fn write_config(&self, text: &str) {
        let f = self.home.join(".config/venv/config.toml");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, text).unwrap();
    }

    /// A uv environment that looks installed: no uv needed to activate it.
    pub fn fake_uv(&self, name: &str) -> PathBuf {
        let dir = self.venvs.join(name);
        fs::create_dir_all(dir.join(".venv/bin")).unwrap();
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        fs::write(dir.join(".venv/pyvenv.cfg"), "version_info = 3.12.4\n").unwrap();
        executable(
            &dir.join(".venv/bin/python"),
            "#!/bin/sh\necho fake python\n",
        );
        dir.join(".venv")
    }

    /// A pixi environment plus a fake `pixi` on PATH whose `shell-hook --json`
    /// prepends the env's bin and sets a few conda/pixi variables. Every
    /// shell-hook call is appended to `pixi-calls.log`.
    pub fn fake_pixi(&self, name: &str) -> PathBuf {
        let dir = self.venvs.join(name);
        let prefix = dir.join(".pixi/envs/default");
        fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::write(
            prefix.join("conda-meta/python-3.13.1-h0_cpython.json"),
            "{}",
        )
        .unwrap();
        fs::write(dir.join("pixi.toml"), "[workspace]\nname = \"x\"\n").unwrap();
        fs::write(dir.join("pixi.lock"), "version: 6\n").unwrap();
        let log = self.root.join("pixi-calls.log");
        executable(
            &self.bin.join("pixi"),
            &format!(
                r#"#!/bin/sh
# fake pixi: only `shell-hook --json --manifest-path M` does anything
[ "$1" = shell-hook ] || exit 0
m=$4
prefix="$(dirname "$m")/.pixi/envs/default"
echo "$m" >> '{log}'
printf '{{"environment_variables":{{"PATH":"%s/bin:%s","CONDA_PREFIX":"%s","CONDA_SHLVL":"1","PIXI_IN_SHELL":"1"}}}}\n' "$prefix" "$PATH" "$prefix"
"#,
                log = log.display()
            ),
        );
        prefix
    }

    pub fn pixi_calls(&self) -> usize {
        fs::read_to_string(self.root.join("pixi-calls.log"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
}

pub fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[track_caller]
pub fn ok(o: &Output) -> String {
    assert!(
        o.status.success(),
        "exit {:?}\n--- stdout\n{}\n--- stderr\n{}",
        o.status.code(),
        stdout(o),
        stderr(o)
    );
    stdout(o)
}

/// Shell snippet that dumps the full environment between markers, so a test
/// can compare snapshots exactly. NUL-separated because values may contain
/// newlines; awk because BSD `env` has no `-0`.
pub const DUMP: &str = "printf '\\n@@DUMP@@\\n'; \
     awk 'BEGIN { for (k in ENVIRON) printf \"%s=%s%c\", k, ENVIRON[k], 0 }'; \
     printf '\\n@@END@@\\n'";

/// The environment snapshots printed by [`DUMP`], in order. `_` (last
/// command) and `SHLVL` change by themselves and are dropped.
pub fn dumps(out: &str) -> Vec<BTreeMap<String, String>> {
    out.split("\n@@DUMP@@\n")
        .skip(1)
        .map(|chunk| {
            let body = chunk.split("\n@@END@@\n").next().unwrap();
            body.split('\0')
                .filter_map(|kv| kv.split_once('='))
                .filter(|(k, _)| *k != "_" && *k != "SHLVL")
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .collect()
}
