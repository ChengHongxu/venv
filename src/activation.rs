//! Activation as pure computation on an environment map: apply an [`Activation`]
//! to a base environment, restore the base from the recorded [`State`], and diff
//! two maps into the changes a shell has to make. `switch`, `off` and `run` all
//! go through here, so they behave the same and can be tested without a shell.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::EnvMap;

/// Exported while an environment is active; nested shells inherit it.
pub const STATE_VAR: &str = "_VENV_STATE";
/// The active environment's name, for prompts (starship `[env_var.VENV_NAME]`).
pub const NAME_VAR: &str = "VENV_NAME";

/// What activating an environment changes, independent of the shell it runs in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    pub prefix: PathBuf,
    /// `PATH`-like variables: entries put in front of the current value.
    pub prepend: BTreeMap<String, Vec<String>>,
    /// Variables set to a fixed value.
    pub set: BTreeMap<String, String>,
    /// Variables removed (`PYTHONHOME`).
    pub unset: Vec<String>,
}

impl Activation {
    /// uv and python-venv: what `bin/activate` does, minus the shell functions.
    pub fn virtualenv(prefix: &Path) -> Activation {
        Activation {
            prefix: prefix.to_path_buf(),
            prepend: [(
                "PATH".into(),
                vec![prefix.join("bin").display().to_string()],
            )]
            .into(),
            set: [("VIRTUAL_ENV".into(), prefix.display().to_string())].into(),
            unset: vec!["PYTHONHOME".into()],
        }
    }
}

/// Recorded in [`STATE_VAR`] so the activation can be undone exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub name: String,
    pub prefix: PathBuf,
    /// Entries this activation put in front of each `PATH`-like variable.
    pub prepend: BTreeMap<String, Vec<String>>,
    /// Those variables' values before activation (`None`: unset).
    pub original: BTreeMap<String, Option<String>>,
    /// Every other variable activation set or unset, with its previous value.
    pub saved: BTreeMap<String, Option<String>>,
}

impl State {
    pub fn from_env(env: &EnvMap) -> Option<Result<State>> {
        env.get(STATE_VAR)
            .map(|s| serde_json::from_str(s).context("_VENV_STATE 无法解析"))
    }
}

fn join(entries: &[String], rest: Option<&str>) -> String {
    match rest.filter(|r| !r.is_empty()) {
        Some(r) => format!("{}:{r}", entries.join(":")),
        None => entries.join(":"),
    }
}

/// zsh's `typeset -U path` keeps only the first occurrence of each entry.
fn dedup(value: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    value
        .split(':')
        .filter(|p| seen.insert(*p))
        .collect::<Vec<_>>()
        .join(":")
}

/// `base` with `act` applied, plus the state that undoes it.
pub fn apply(base: &EnvMap, name: &str, act: &Activation) -> EnvMap {
    let mut env = base.clone();
    let mut original = BTreeMap::new();
    let mut saved = BTreeMap::new();
    for (key, entries) in &act.prepend {
        let old = base.get(key);
        original.insert(key.clone(), old.cloned());
        env.insert(key.clone(), join(entries, old.map(String::as_str)));
    }
    let mut save = |key: &str, env: &mut EnvMap| {
        if !act.prepend.contains_key(key) {
            saved
                .entry(key.to_string())
                .or_insert_with(|| base.get(key).cloned());
        }
        env.remove(key);
    };
    for key in &act.unset {
        save(key, &mut env);
    }
    for (key, value) in &act.set {
        save(key, &mut env);
        env.insert(key.clone(), value.clone());
    }
    save(NAME_VAR, &mut env);
    env.insert(NAME_VAR.into(), name.to_string());
    let state = State {
        name: name.to_string(),
        prefix: act.prefix.clone(),
        prepend: act.prepend.clone(),
        original,
        saved,
    };
    env.insert(
        STATE_VAR.into(),
        serde_json::to_string(&state).expect("state serializes"),
    );
    env
}

/// One `PATH`-like variable put back. Unchanged since activation (allowing for
/// `typeset -U` dedup): the exact original. Otherwise remove the first occurrence
/// of each entry we added and keep everything else, including what the user or
/// a nested shell's rc files added since.
fn restore_list(
    current: Option<&str>,
    entries: &[String],
    original: Option<&str>,
) -> Option<String> {
    let after = join(entries, original);
    let Some(current) = current else {
        return original.map(str::to_string);
    };
    if current == after || current == dedup(&after) {
        return original.map(str::to_string);
    }
    let mut parts: Vec<&str> = current.split(':').collect();
    for entry in entries {
        if let Some(i) = parts.iter().position(|p| p == entry) {
            parts.remove(i);
        }
    }
    let rest = parts.join(":");
    if rest.is_empty() && original.is_none_or(str::is_empty) {
        original.map(str::to_string)
    } else {
        Some(rest)
    }
}

fn put(env: &mut EnvMap, key: &str, value: Option<String>) {
    match value {
        Some(v) => env.insert(key.to_string(), v),
        None => env.remove(key),
    };
}

/// The environment as it was before `state`'s activation.
pub fn restore(current: &EnvMap, state: &State) -> EnvMap {
    let mut env = current.clone();
    for (key, entries) in &state.prepend {
        let original = state.original.get(key).cloned().flatten();
        let value = restore_list(
            current.get(key).map(String::as_str),
            entries,
            original.as_deref(),
        );
        put(&mut env, key, value);
    }
    for (key, value) in &state.saved {
        put(&mut env, key, value.clone());
    }
    env.remove(STATE_VAR);
    env
}

/// Nested shell: rc files (or macOS path_helper) may have pushed other entries
/// in front. Move ours back to the front; `None` when already in place.
pub fn relocate(current: &EnvMap, state: &State) -> Option<EnvMap> {
    let mut env = current.clone();
    let mut changed = false;
    for (key, entries) in &state.prepend {
        let value = current.get(key).map(String::as_str).unwrap_or_default();
        let prefix = entries.join(":");
        if value == prefix || value.starts_with(&format!("{prefix}:")) {
            continue;
        }
        let mut parts: Vec<&str> = value.split(':').filter(|p| !p.is_empty()).collect();
        parts.retain(|p| !entries.iter().any(|e| e == p));
        env.insert(key.clone(), join(entries, Some(&parts.join(":"))));
        changed = true;
    }
    changed.then_some(env)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Set(String, String),
    Unset(String),
}

pub fn valid_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What a shell must do to go from `from` to `to`, sorted by variable name.
pub fn diff(from: &EnvMap, to: &EnvMap) -> Vec<Change> {
    let keys: std::collections::BTreeSet<&String> = from.keys().chain(to.keys()).collect();
    keys.into_iter()
        .filter(|k| valid_name(k))
        .filter_map(|k| match (from.get(k), to.get(k)) {
            (a, Some(b)) if a != Some(b) => Some(Change::Set(k.clone(), b.clone())),
            (Some(_), None) => Some(Change::Unset(k.clone())),
            _ => None,
        })
        .collect()
}

/// Turn the variables `pixi shell-hook --json` reports into an [`Activation`]
/// relative to `base` (the environment pixi ran in). A value of the form
/// `new:entries:<old value>` is a prepend, which keeps the result valid for any
/// later PATH; anything else is set as is.
pub fn classify(
    base: &EnvMap,
    vars: &BTreeMap<String, String>,
    default_prefix: &Path,
) -> Activation {
    let mut act = Activation {
        prefix: vars
            .get("CONDA_PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_prefix.to_path_buf()),
        ..Default::default()
    };
    for (key, value) in vars {
        if !valid_name(key) || key == STATE_VAR || key == NAME_VAR {
            continue;
        }
        let old = base.get(key);
        if old == Some(value) {
            continue;
        }
        let prepended = old
            .filter(|o| !o.is_empty())
            .and_then(|o| value.strip_suffix(o.as_str()))
            .and_then(|head| head.strip_suffix(':'))
            .map(|head| head.split(':').map(str::to_string).collect::<Vec<_>>())
            .filter(|entries| !entries.is_empty() && entries.iter().all(|e| !e.is_empty()));
        match prepended {
            Some(entries) => {
                act.prepend.insert(key.clone(), entries);
            }
            None => {
                act.set.insert(key.clone(), value.clone());
            }
        }
    }
    if !act.set.contains_key("PYTHONHOME") {
        act.unset.push("PYTHONHOME".into());
    }
    act
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

    fn state(e: &EnvMap) -> State {
        State::from_env(e).unwrap().unwrap()
    }

    fn venv(prefix: &str) -> Activation {
        Activation::virtualenv(Path::new(prefix))
    }

    #[test]
    fn apply_then_restore_is_exact() {
        let base = env(&[
            ("PATH", "/my bin:/usr/bin:/bin"),
            ("PYTHONHOME", "/usr"),
            ("VIRTUAL_ENV", ""),
            ("KEEP", "x"),
        ]);
        let on = apply(&base, "a", &venv("/v/a b/.venv"));
        assert_eq!(on["PATH"], "/v/a b/.venv/bin:/my bin:/usr/bin:/bin");
        assert_eq!(on["VIRTUAL_ENV"], "/v/a b/.venv");
        assert_eq!(on[NAME_VAR], "a");
        assert!(!on.contains_key("PYTHONHOME"));
        assert_eq!(restore(&on, &state(&on)), base);
    }

    #[test]
    fn restore_handles_unset_path_and_switching() {
        let base = env(&[("HOME", "/h")]);
        let on = apply(&base, "a", &venv("/a"));
        assert_eq!(on["PATH"], "/a/bin");
        assert_eq!(restore(&on, &state(&on)), base);

        // a -> b -> off, through the base each time
        let base = env(&[("PATH", "/usr/bin")]);
        let a = apply(&base, "a", &venv("/a"));
        let b = apply(&restore(&a, &state(&a)), "b", &venv("/b"));
        assert_eq!(b["PATH"], "/b/bin:/usr/bin");
        assert_eq!(b["VIRTUAL_ENV"], "/b");
        assert_eq!(restore(&b, &state(&b)), base);
    }

    #[test]
    fn restore_after_typeset_u_dedup() {
        // env's bin was already on PATH; zsh `typeset -U path` drops the later copy
        let base = env(&[("PATH", "/usr/bin:/a/bin:/bin")]);
        let mut on = apply(&base, "a", &venv("/a"));
        on.insert("PATH".into(), "/a/bin:/usr/bin:/bin".into());
        assert_eq!(restore(&on, &state(&on))["PATH"], "/usr/bin:/a/bin:/bin");
    }

    #[test]
    fn restore_keeps_entries_added_later() {
        let base = env(&[("PATH", "/usr/bin")]);
        let mut on = apply(&base, "a", &venv("/a"));
        // user appended, and path_helper moved system dirs in front
        on.insert("PATH".into(), "/usr/bin:/a/bin:/opt/tool".into());
        assert_eq!(restore(&on, &state(&on))["PATH"], "/usr/bin:/opt/tool");
        // a user-added duplicate of our entry survives: only the first goes
        on.insert("PATH".into(), "/a/bin:/usr/bin:/a/bin".into());
        assert_eq!(restore(&on, &state(&on))["PATH"], "/usr/bin:/a/bin");
    }

    #[test]
    fn relocate_moves_entries_back_to_front() {
        let base = env(&[("PATH", "/usr/bin")]);
        let on = apply(&base, "a", &venv("/a"));
        assert_eq!(relocate(&on, &state(&on)), None);

        let mut nested = on.clone();
        nested.insert("PATH".into(), "/h/.pixi/bin:/a/bin:/usr/bin".into());
        let fixed = relocate(&nested, &state(&nested)).unwrap();
        assert_eq!(fixed["PATH"], "/a/bin:/h/.pixi/bin:/usr/bin");
        // off in the nested shell keeps what its rc files added
        assert_eq!(
            restore(&fixed, &state(&fixed))["PATH"],
            "/h/.pixi/bin:/usr/bin"
        );
    }

    #[test]
    fn diff_lists_sets_and_unsets() {
        let from = env(&[("A", "1"), ("B", "2"), ("bad-name", "x")]);
        let to = env(&[("A", "1"), ("C", "3"), ("bad-name", "y")]);
        assert_eq!(
            diff(&from, &to),
            [
                Change::Unset("B".into()),
                Change::Set("C".into(), "3".into())
            ]
        );
    }

    #[test]
    fn classify_pixi_output() {
        let base = env(&[
            ("PATH", "/usr/bin:/bin"),
            ("CONDA_SHLVL", "0"),
            ("X", "same"),
        ]);
        let vars: BTreeMap<String, String> = [
            ("PATH", "/p/.pixi/envs/default/bin:/usr/bin:/bin"),
            ("CONDA_PREFIX", "/p/.pixi/envs/default"),
            ("CONDA_SHLVL", "1"),
            ("X", "same"),
            ("PIXI_PROMPT", "(p) "),
            ("1BAD", "x"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let act = classify(&base, &vars, Path::new("/fallback"));
        assert_eq!(act.prefix, PathBuf::from("/p/.pixi/envs/default"));
        assert_eq!(act.prepend["PATH"], ["/p/.pixi/envs/default/bin"]);
        assert_eq!(act.set["CONDA_SHLVL"], "1");
        assert_eq!(act.set["PIXI_PROMPT"], "(p) ");
        assert!(!act.set.contains_key("X") && !act.set.contains_key("1BAD"));
        assert_eq!(act.unset, ["PYTHONHOME"]);

        // Not a pure prepend (pixi reordered): set as a whole
        let vars = [("PATH".to_string(), "/bin:/x".to_string())].into();
        let act = classify(&base, &vars, Path::new("/fallback"));
        assert_eq!(act.set["PATH"], "/bin:/x");
        assert_eq!(act.prefix, PathBuf::from("/fallback"));
    }

    #[test]
    fn pixi_round_trip_restores_conda_vars() {
        let base = env(&[("PATH", "/usr/bin"), ("CONDA_PREFIX", "/conda/base")]);
        let vars: BTreeMap<String, String> = [
            ("PATH", "/p/bin:/usr/bin"),
            ("CONDA_PREFIX", "/p"),
            ("CONDA_ENV_SHLVL_1_CONDA_PREFIX", "/conda/base"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let act = classify(&base, &vars, Path::new("/p"));
        let on = apply(&base, "p", &act);
        assert_eq!(on["CONDA_PREFIX"], "/p");
        assert_eq!(restore(&on, &state(&on)), base);
    }
}
