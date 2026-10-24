//! Shell-specific code: quoting, `export`/`unset`, and the integration printed by
//! `venv hook`. zsh and bash share POSIX quoting; a fish variant would add its
//! own arms here and its own `hook.fish`.

use crate::activation::Change;

const ZSH_HOOK: &str = include_str!("hook.zsh");
const BASH_HOOK: &str = include_str!("hook.bash");

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    Zsh,
    Bash,
}

impl Shell {
    pub fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
        }
    }

    /// Single-quoted; `'` becomes `'\''`. Spaces, `$`, backticks, `\`, `!`,
    /// newlines and non-ASCII all survive `eval` unchanged.
    pub fn quote(self, s: &str) -> String {
        match self {
            Shell::Zsh | Shell::Bash => format!("'{}'", s.replace('\'', r"'\''")),
        }
    }

    fn rehash(self) -> &'static str {
        match self {
            Shell::Zsh => "rehash",
            Shell::Bash => "hash -r",
        }
    }

    /// Code that applies `changes`. Empty when there is nothing to do.
    pub fn render(self, changes: &[Change]) -> String {
        let mut out = String::new();
        for change in changes {
            match change {
                Change::Set(k, v) => out.push_str(&format!("export {k}={}\n", self.quote(v))),
                Change::Unset(k) => out.push_str(&format!("unset {k}\n")),
            }
        }
        let path_changed = changes.iter().any(|c| match c {
            Change::Set(k, _) | Change::Unset(k) => k == "PATH",
        });
        if path_changed {
            out.push_str(self.rehash());
            out.push('\n');
        }
        out
    }

    /// The integration evaluated by `eval "$(venv hook <shell>)"`. `bin` is the
    /// real executable, so later calls skip the pixi global trampoline.
    pub fn hook(self, bin: &str) -> String {
        let template = match self {
            Shell::Zsh => ZSH_HOOK,
            Shell::Bash => BASH_HOOK,
        };
        template.replace("@VENV_BIN@", &self.quote(bin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Values that break naive quoting.
    const NASTY: &[&str] = &[
        "plain",
        "with space",
        "it's",
        "'",
        "$HOME `id` $(id) \\n \\",
        "!event",
        "line1\nline2",
        "中文 路径",
        "",
        "a:b:c",
    ];

    #[test]
    fn quote_survives_real_shells() {
        for shell in [Shell::Zsh, Shell::Bash] {
            let bin = shell.name();
            if Command::new(bin).arg("-c").arg("true").output().is_err() {
                eprintln!("skip: {bin} not installed");
                continue;
            }
            for value in NASTY {
                let changes = [Change::Set("V".into(), value.to_string())];
                let script = format!("{}printf '%s' \"$V\"", shell.render(&changes));
                let out = Command::new(bin).arg("-c").arg(&script).output().unwrap();
                assert!(out.status.success(), "{bin}: {script}");
                assert_eq!(String::from_utf8(out.stdout).unwrap(), *value, "{bin}");
            }
        }
    }

    #[test]
    fn render_rehashes_only_when_path_changes() {
        let set = |k: &str| Change::Set(k.into(), "x".into());
        assert_eq!(Shell::Zsh.render(&[]), "");
        assert_eq!(Shell::Zsh.render(&[set("A")]), "export A='x'\n");
        assert_eq!(
            Shell::Bash.render(&[set("PATH"), Change::Unset("B".into())]),
            "export PATH='x'\nunset B\nhash -r\n"
        );
    }

    #[test]
    fn hook_embeds_quoted_binary() {
        let h = Shell::Zsh.hook("/opt/my env/bin/venv");
        assert!(h.contains("__venv_bin='/opt/my env/bin/venv'"));
        assert!(!h.contains("@VENV_BIN@"));
        assert!(!h.contains('\r'));
        assert!(Shell::Bash.hook("/x").contains("complete -F"));
    }
}
