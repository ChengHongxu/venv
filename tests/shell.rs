//! End to end through real zsh and bash: `eval "$(venv hook ...)"`, then the
//! shell function. Environments are faked on disk (and pixi by a stub script),
//! so these run offline and fast.

mod common;

use common::*;

const SHELLS: &[&str] = &["zsh", "bash"];

/// A child shell that reads no rc files of the machine running the tests.
fn no_rc(shell: &str) -> &'static str {
    if shell == "zsh" {
        "zsh -f"
    } else {
        "bash --norc --noprofile"
    }
}

fn have(shell: &str) -> bool {
    let found = std::process::Command::new(shell)
        .args(["-c", "true"])
        .output()
        .is_ok();
    if !found {
        assert!(
            std::env::var_os("VENV_REQUIRE_BACKENDS").is_none(),
            "{shell} is required"
        );
        eprintln!("skip: {shell} not installed");
    }
    found
}

#[test]
fn switch_then_off_restores_everything() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        // Spaces and a quote in every path venv touches
        let s = Sandbox::with_venvs_dir("my envs/it's");
        let a = s.fake_uv("a");
        let b = s.fake_uv("b");
        let script = format!(
            "export PYTHONHOME=/usr NASTY=\"$(printf 'x\\ny $HOME `q` \\x27')\"\n\
             {DUMP}\nvenv switch a\n{DUMP}\nvenv use b\n{DUMP}\nvenv off\n{DUMP}"
        );
        let out = ok(&s.shell(shell, &script));
        let d = dumps(&out);
        assert_eq!(d.len(), 4, "{shell}: {out}");
        let (before, on_a, on_b, after) = (&d[0], &d[1], &d[2], &d[3]);

        assert_eq!(on_a["VENV_NAME"], "a", "{shell}");
        assert_eq!(on_a["VIRTUAL_ENV"], a.display().to_string());
        assert_eq!(
            on_a["PATH"],
            format!("{}/bin:{}", a.display(), before["PATH"])
        );
        assert!(!on_a.contains_key("PYTHONHOME"));
        assert_eq!(on_a["NASTY"], before["NASTY"]);

        assert_eq!(on_b["VENV_NAME"], "b");
        assert_eq!(on_b["VIRTUAL_ENV"], b.display().to_string());
        assert_eq!(
            on_b["PATH"],
            format!("{}/bin:{}", b.display(), before["PATH"])
        );

        assert_eq!(
            after, before,
            "{shell}: off must restore the exact environment"
        );
    }
}

#[test]
fn messages_go_to_stderr_only() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        let s = Sandbox::new();
        s.fake_uv("a");
        let out = s.shell(shell, "venv switch a; echo MARK; venv off; echo MARK2");
        assert_eq!(ok(&out), "MARK\nMARK2\n", "{shell}");
        let err = stderr(&out);
        assert!(err.contains("已切换到 a"), "{shell}: {err}");
        assert!(err.contains("Python  3.12.4"), "{shell}: {err}");
        assert!(err.contains("已退出 a"), "{shell}: {err}");
    }
}

#[test]
fn failed_switch_and_help_change_nothing() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        let s = Sandbox::new();
        s.fake_uv("a");
        let script = format!(
            "venv switch a\n{DUMP}\n\
             venv switch nope; echo \"rc=$?\"\n{DUMP}\n\
             venv switch --help; echo \"rc=$?\"\n{DUMP}"
        );
        let out = s.shell(shell, &script);
        let text = ok(&out);
        let d = dumps(&text);
        assert_eq!(d[1], d[0], "{shell}: failed switch changed the environment");
        assert_eq!(d[2], d[0], "{shell}: --help changed the environment");
        assert!(text.contains("rc=1"), "{shell}: {text}");
        assert!(text.contains("rc=0"), "{shell}: {text}");
        let err = stderr(&out);
        assert!(err.contains("环境 'nope' 不存在"), "{shell}: {err}");
        assert!(
            err.contains("用法:"),
            "{shell}: help should be on stderr: {err}"
        );
    }
}

#[test]
fn pixi_activation_is_cached_and_restored() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        let s = Sandbox::new();
        let prefix = s.fake_pixi("p");
        s.fake_uv("u");
        let script = format!(
            "{DUMP}\nvenv switch p\n{DUMP}\nvenv switch u\nvenv switch p\nvenv off\n{DUMP}\n\
             if typeset -f pixi >/dev/null 2>&1; then echo HAS_PIXI_FUNCTION; fi"
        );
        let out = ok(&s.shell(shell, &script));
        let d = dumps(&out);
        let (before, on, after) = (&d[0], &d[1], &d[2]);
        assert_eq!(on["CONDA_PREFIX"], prefix.display().to_string(), "{shell}");
        assert_eq!(on["CONDA_SHLVL"], "1");
        assert_eq!(on["PIXI_IN_SHELL"], "1");
        assert_eq!(on["VENV_NAME"], "p");
        assert_eq!(
            on["PATH"],
            format!("{}/bin:{}", prefix.display(), before["PATH"])
        );
        assert_eq!(after, before, "{shell}");
        assert!(
            !out.contains("HAS_PIXI_FUNCTION"),
            "{shell}: no pixi() wrapper"
        );
        // Two switches to p, one pixi run: the second came from the cache
        assert_eq!(s.pixi_calls(), 1, "{shell}");
    }
}

#[test]
fn pixi_cache_follows_the_lock_file() {
    let s = Sandbox::new();
    s.fake_pixi("p");
    let switch = |s: &Sandbox| ok(&s.shell("bash", "venv switch p"));
    switch(&s);
    switch(&s);
    assert_eq!(s.pixi_calls(), 1, "second shell should hit the cache");
    std::fs::write(s.venvs.join("p/pixi.lock"), "version: 6\nchanged: true\n").unwrap();
    switch(&s);
    assert_eq!(s.pixi_calls(), 2, "a changed pixi.lock must invalidate");
    std::fs::write(s.venvs.join("p/pixi.toml"), "[workspace]\nname = \"y\"\n").unwrap();
    switch(&s);
    assert_eq!(s.pixi_calls(), 3, "a changed pixi.toml must invalidate");
}

#[test]
fn nested_shell_puts_env_bin_back_in_front() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        let s = Sandbox::new();
        let a = s.fake_uv("a");
        let inner = s.root.join("inner.sh");
        let bin = a.join("bin").display().to_string();
        // The child's rc pushes ~/.pixi/bin in front (like macOS path_helper or
        // the dotfiles' `path=(...)`), then loads venv
        std::fs::write(
            &inner,
            format!(
                "export PATH=\"$HOME/.pixi/bin:$PATH\"\n\
                 eval \"$('{BIN}' hook {shell})\"\n\
                 echo \"NESTED=$PATH\"\nvenv off\necho \"OFF=$PATH\"\n"
            ),
        )
        .unwrap();
        let script = format!(
            "venv switch a\necho \"OUTER=$PATH\"\n{} {}\n",
            no_rc(shell),
            inner.display()
        );
        let out = ok(&s.shell(shell, &script));
        let line = |tag: &str| {
            out.lines()
                .find_map(|l| l.strip_prefix(tag))
                .unwrap_or_else(|| panic!("{shell}: no {tag} in {out}"))
                .to_string()
        };
        let outer = line("OUTER=");
        let base = outer.strip_prefix(&format!("{bin}:")).unwrap();
        let pixi_bin = s.home.join(".pixi/bin").display().to_string();
        assert_eq!(
            line("NESTED="),
            format!("{bin}:{pixi_bin}:{base}"),
            "{shell}"
        );
        // off in the child keeps what the child's own rc added
        assert_eq!(line("OFF="), format!("{pixi_bin}:{base}"), "{shell}");
    }
}

#[test]
fn zsh_typeset_u_dedup_still_restores() {
    if !have("zsh") {
        return;
    }
    let s = Sandbox::new();
    let a = s.fake_uv("a");
    // env's bin already on PATH; typeset -U drops the later copy on activation
    let script = format!(
        "typeset -U path PATH\nexport PATH=\"/usr/bin:{}/bin:/bin\"\n{DUMP}\nvenv switch a\n{DUMP}\nvenv off\n{DUMP}",
        a.display()
    );
    let out = ok(&s.shell("zsh", &script));
    let d = dumps(&out);
    assert_eq!(d[1]["PATH"], format!("{}/bin:/usr/bin:/bin", a.display()));
    assert_eq!(d[2], d[0]);
}

#[test]
fn startup_auto_activates_and_survives_a_missing_env() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        let s = Sandbox::new();
        s.fake_uv("default");
        s.write_config("auto_activate = \"default\"\n");
        let out = s.shell(shell, "echo \"NAME=$VENV_NAME\"");
        assert_eq!(ok(&out), "NAME=default\n", "{shell}");
        assert_eq!(stderr(&out), "", "{shell}: auto-activation is silent");

        // Already active (inherited): startup leaves it alone
        let out = s.shell(
            shell,
            "venv switch default 2>/dev/null; echo \"NAME=$VENV_NAME\"",
        );
        assert_eq!(ok(&out), "NAME=default\n");

        s.write_config("auto_activate = \"gone\"\n");
        let out = s.shell(shell, "echo \"ALIVE NAME=${VENV_NAME:-}\"");
        assert_eq!(ok(&out), "ALIVE NAME=\n", "{shell}");
        assert!(
            stderr(&out).contains("auto_activate"),
            "{shell}: {}",
            stderr(&out)
        );

        // A broken config must not break the shell either
        s.write_config("auto_activate = [\n");
        let out = s.shell(shell, "echo ALIVE");
        assert_eq!(ok(&out), "ALIVE\n", "{shell}");
    }
}

#[test]
fn startup_drops_state_of_a_deleted_env() {
    for shell in SHELLS.iter().copied().filter(|s| have(s)) {
        let s = Sandbox::new();
        let a = s.fake_uv("a");
        let inner = s.root.join("inner.sh");
        std::fs::write(
            &inner,
            format!(
                "eval \"$('{BIN}' hook {shell})\"\necho \"NAME=${{VENV_NAME:-}} STATE=${{_VENV_STATE:-}}\"\n"
            ),
        )
        .unwrap();
        let script = format!(
            "venv switch a 2>/dev/null\nrm -rf '{}'\n{} {}",
            a.display(),
            no_rc(shell),
            inner.display()
        );
        let out = s.shell(shell, &script);
        assert_eq!(ok(&out), "NAME= STATE=\n", "{shell}");
        assert!(stderr(&out).contains("已经不存在"), "{shell}");
    }
}

#[test]
fn without_the_hook_switch_explains_what_to_do() {
    let s = Sandbox::new();
    s.fake_uv("a");
    let out = s.venv(&["switch", "a"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("venv init"), "{}", stderr(&out));

    ok(&s.venv(&["init", "zsh"]));
    let out = s.venv(&["off"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("source ~/.zshrc"), "{}", stderr(&out));

    // Listing needs no shell integration
    let out = s.venv(&["switch"]);
    assert!(out.status.success());
    assert!(stderr(&out).contains("a "), "{}", stderr(&out));
}

#[test]
fn init_writes_the_hook_line_once_and_through_symlinks() {
    let s = Sandbox::new();
    let rc = s.home.join(".zshrc");
    std::fs::write(&rc, "export A=1").unwrap();
    ok(&s.venv(&["init", "zsh"]));
    ok(&s.venv(&["init", "zsh"]));
    assert_eq!(
        std::fs::read_to_string(&rc).unwrap(),
        "export A=1\n\n# venv\neval \"$(venv hook zsh)\"\n"
    );

    // ~/.bashrc symlinked from a dotfiles repo: the link stays, the target changes
    let dots = s.root.join("dotfiles");
    std::fs::create_dir_all(&dots).unwrap();
    std::fs::write(dots.join("bashrc"), "# mine\n").unwrap();
    std::os::unix::fs::symlink(dots.join("bashrc"), s.home.join(".bashrc")).unwrap();
    let out = s.venv(&["init", "bash"]);
    ok(&out);
    assert!(
        s.home
            .join(".bashrc")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(dots.join("bashrc")).unwrap(),
        "# mine\n\n# venv\neval \"$(venv hook bash)\"\n"
    );
    assert!(stderr(&out).contains("dotfiles/bashrc"), "{}", stderr(&out));
}

#[test]
fn list_path_which_and_run() {
    let s = Sandbox::new();
    let u = s.fake_uv("u");
    s.fake_pixi("p");
    std::fs::create_dir_all(s.venvs.join("bare/bin")).unwrap();
    std::fs::write(s.venvs.join("bare/pyvenv.cfg"), "version = 3.12\n").unwrap();

    assert_eq!(ok(&s.venv(&["list", "--names"])), "p\nu\n");
    let table = ok(&s.venv(&["list"]));
    assert!(table.contains("p  pixi  3.13.1"), "{table}");
    assert!(table.contains("u  uv    3.12.4"), "{table}");
    let json: serde_json::Value = serde_json::from_str(&ok(&s.venv(&["list", "--json"]))).unwrap();
    assert_eq!(json[1]["name"], "u");
    assert_eq!(json[1]["python"], "3.12.4");
    assert_eq!(json[1]["current"], false);

    assert_eq!(ok(&s.venv(&["path", "u"])).trim(), u.display().to_string());
    assert_eq!(
        ok(&s.venv(&["which", "-e", "u", "python"])).trim(),
        u.join("bin/python").display().to_string()
    );
    assert!(!s.venv(&["which", "-e", "u", "nope"]).status.success());

    let out = ok(&s.venv(&["run", "-e", "u", "--", "python"]));
    assert_eq!(out, "fake python\n");
    let out = ok(&s.venv(&[
        "run",
        "-e",
        "u",
        "--",
        "sh",
        "-c",
        "echo $VENV_NAME $VIRTUAL_ENV",
    ]));
    assert_eq!(out.trim(), format!("u {}", u.display()));
}

#[test]
fn rm_through_a_symlinked_venvs_home() {
    // dotfiles layout: ~/.venvs -> <repo>/.venvs, so pixi.toml resolves to a
    // path that does not start with $VENVS_HOME
    let mut s = Sandbox::new();
    s.fake_pixi("p");
    let link = s.root.join("linked-venvs");
    std::os::unix::fs::symlink(&s.venvs, &link).unwrap();
    s.extra
        .push(("VENVS_HOME".into(), link.display().to_string()));

    // Activate once so there is a cached activation for rm to delete
    ok(&s.shell("bash", "venv switch p"));
    let cache = s.home.join(".cache/venv/pixi/p.json");
    assert!(cache.is_file(), "switch should have cached the activation");

    let out = s.venv(&["rm", "p", "--yes"]);
    ok(&out);
    assert!(stderr(&out).contains("已删除 p"), "{}", stderr(&out));
    assert_eq!(ok(&s.venv(&["list", "--names"])), "");
    assert!(!s.venvs.join("p").exists());
    assert!(!cache.exists(), "the cached activation is removed too");
    assert!(link.is_dir(), "the VENVS_HOME link itself stays");
}

#[test]
fn help_is_chinese() {
    let s = Sandbox::new();
    let out = ok(&s.venv(&["--help"]));
    for word in ["用法:", "子命令", "选项", "显示帮助", "创建环境并激活"] {
        assert!(out.contains(word), "missing {word}: {out}");
    }
    assert!(
        !out.contains("Usage:") && !out.contains("Options:"),
        "{out}"
    );
    let out = ok(&s.venv(&["new", "--help"]));
    assert!(out.contains("参数") && out.contains("--no-switch"), "{out}");
}
