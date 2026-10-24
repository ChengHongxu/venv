//! The three backends with the real pixi, uv and python: new, add, switch, off,
//! rm. Each test is skipped when its tool is missing, unless
//! VENV_REQUIRE_BACKENDS is set (CI), where a missing tool fails the test.
//! They download packages, so they share pixi's and uv's real caches.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use common::*;

/// One backend test at a time: they hit the network and shared caches.
static SERIAL: Mutex<()> = Mutex::new(());

fn require(found: Option<PathBuf>, what: &str) -> Option<PathBuf> {
    if found.is_none() {
        assert!(
            std::env::var_os("VENV_REQUIRE_BACKENDS").is_none(),
            "{what} is required (VENV_REQUIRE_BACKENDS is set)"
        );
        eprintln!("skip: {what} not found");
    }
    found
}

fn real_home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap())
}

/// `name` on the real PATH, or in its usual install directory.
fn tool(name: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| PathBuf::from(d).join(name))
        .chain([real_home().join(fallback)])
        .find(|p| p.is_file())
}

fn capture(cmd: &PathBuf, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A sandbox whose HOME is fresh, but with the real download caches, and the
/// tools' directories on PATH.
fn sandbox(tools: &[&PathBuf]) -> Sandbox {
    let mut s = Sandbox::new();
    for t in tools {
        s.path = format!("{}:{}", t.parent().unwrap().display(), s.path);
    }
    let pixi_cache = real_home().join(".cache/rattler/cache");
    s.extra
        .push(("PIXI_CACHE_DIR".into(), pixi_cache.display().to_string()));
    if let Some(uv) = tool("uv", ".local/bin/uv") {
        if let Some(dir) = capture(&uv, &["cache", "dir"]) {
            s.extra.push(("UV_CACHE_DIR".into(), dir));
        }
        if let Some(dir) = capture(&uv, &["python", "dir"]) {
            s.extra.push(("UV_PYTHON_INSTALL_DIR".into(), dir));
        }
    }
    for var in [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
    ] {
        if let Ok(v) = std::env::var(var) {
            s.extra.push((var.into(), v));
        }
    }
    s
}

/// new → add → switch (import works, python is the env's) → off → rm
fn round_trip(s: &Sandbox, name: &str, new_args: &[&str], pkg: &str, declared_in: &str) {
    let mut args = vec!["new", name, "--no-switch"];
    args.extend(new_args);
    ok(&s.venv(&args));
    ok(&s.venv(&["add", "-e", name, pkg]));
    let decl = std::fs::read_to_string(s.venvs.join(name).join(declared_in)).unwrap();
    assert!(
        decl.contains(pkg),
        "{declared_in} should list {pkg}:\n{decl}"
    );

    for shell in ["zsh", "bash"] {
        if Command::new(shell).args(["-c", "true"]).output().is_err() {
            continue;
        }
        let script = format!(
            "{DUMP}\nvenv switch {name}\n\
             python -c 'import sys, {pkg}; print(\"PREFIX=\" + sys.prefix)'\n\
             venv off\n{DUMP}"
        );
        let out = ok(&s.shell(shell, &script));
        let prefix = out
            .lines()
            .find_map(|l| l.strip_prefix("PREFIX="))
            .unwrap_or_else(|| panic!("{shell}: python did not run: {out}"));
        let expected = PathBuf::from(ok(&s.venv(&["path", name])).trim())
            .canonicalize()
            .unwrap();
        assert_eq!(
            PathBuf::from(prefix).canonicalize().unwrap(),
            expected,
            "{shell}"
        );
        let d = dumps(&out);
        assert_eq!(d[1], d[0], "{shell}: off must restore the environment");
    }

    ok(&s.venv(&["remove", "-e", name, pkg]));
    let decl = std::fs::read_to_string(s.venvs.join(name).join(declared_in)).unwrap();
    assert!(
        !decl.contains(pkg),
        "{declared_in} should not list {pkg}:\n{decl}"
    );
    ok(&s.venv(&["rm", name, "--yes"]));
    assert!(!s.venvs.join(name).exists());
}

#[test]
fn pixi() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(pixi) = require(tool("pixi", ".pixi/bin/pixi"), "pixi") else {
        return;
    };
    let s = sandbox(&[&pixi]);
    round_trip(
        &s,
        "px",
        &["--backend", "pixi", "--python", "3.12"],
        "six",
        "pixi.toml",
    );
}

#[test]
fn pixi_manifest_symlinked_from_dotfiles() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(pixi) = require(tool("pixi", ".pixi/bin/pixi"), "pixi") else {
        return;
    };
    let s = sandbox(&[&pixi]);
    let dots = s.root.join("dotfiles/venvs/d");
    std::fs::create_dir_all(&dots).unwrap();
    let platform = capture(&pixi, &["info", "--json"])
        .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
        .and_then(|v| v["platform"].as_str().map(str::to_string))
        .expect("pixi info --json reports the platform");
    std::fs::write(
        dots.join("pixi.toml"),
        format!(
            "[workspace]\nname = \"d\"\nchannels = [\"conda-forge\"]\nplatforms = [\"{platform}\"]\n\n[dependencies]\npython = \"3.12.*\"\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(s.venvs.join("d")).unwrap();
    std::os::unix::fs::symlink(dots.join("pixi.toml"), s.venvs.join("d/pixi.toml")).unwrap();

    ok(&s.venv(&["sync", "d"]));
    ok(&s.venv(&["add", "-e", "d", "six"]));
    // Everything lands next to the real file, nothing in VENVS_HOME
    assert!(
        std::fs::read_to_string(dots.join("pixi.toml"))
            .unwrap()
            .contains("six")
    );
    assert!(dots.join("pixi.lock").is_file());
    assert!(dots.join(".pixi/envs/default/conda-meta").is_dir());
    assert!(!s.venvs.join("d/.pixi").exists() && !s.venvs.join("d/pixi.lock").exists());
    let list = ok(&s.venv(&["list"]));
    assert!(list.contains("→"), "{list}");

    let out = s.venv(&["rm", "d", "--yes"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("dotfiles"), "{}", stderr(&out));
    ok(&s.venv(&["rm", "d", "--yes", "--force"]));
    assert!(!s.venvs.join("d").exists());
    assert!(
        !dots.join(".pixi").exists(),
        "the environment itself is deleted"
    );
    assert!(dots.join("pixi.toml").is_file() && dots.join("pixi.lock").is_file());
}

#[test]
fn uv() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(uv) = require(tool("uv", ".local/bin/uv"), "uv") else {
        return;
    };
    let s = sandbox(&[&uv]);
    round_trip(
        &s,
        "uvenv",
        &["--backend", "uv", "--python", "3.12"],
        "six",
        "pyproject.toml",
    );
}

/// A python whose `-m venv` works (has ensurepip).
fn venv_capable_python() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("VENV_TEST_PYTHON") {
        return Some(PathBuf::from(p));
    }
    let works = |p: &PathBuf| {
        Command::new(p)
            .args(["-c", "import ensurepip, venv"])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let on_path = ["python3.13", "python3.12", "python3"]
        .iter()
        .filter_map(|n| tool(n, ".nonexistent"))
        .find(works);
    on_path.or_else(|| {
        let uv = tool("uv", ".local/bin/uv")?;
        let found = capture(&uv, &["python", "find", "--managed-python", "3.13"])
            .or_else(|| capture(&uv, &["python", "find", "--managed-python", "3.12"]))?;
        Some(PathBuf::from(found)).filter(works)
    })
}

#[test]
fn python_venv() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(python) = require(venv_capable_python(), "a python with ensurepip") else {
        return;
    };
    let s = sandbox(&[]);
    s.write_config(&format!("[venv]\npython = \"{}\"\n", python.display()));
    round_trip(&s, "pv", &["--backend", "venv"], "six", "requirements.txt");
}
