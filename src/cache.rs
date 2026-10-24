//! Cached result of `pixi shell-hook --json`, one file per environment under
//! `~/.cache/venv/pixi/`. The key hashes everything the activation depends on;
//! the stored inputs catch a base environment the result does not fit.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::EnvMap;
use crate::activation::Activation;

const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub version: u32,
    pub key: String,
    pub activation: Activation,
    /// Every variable pixi reported, with its value in the base environment pixi
    /// ran in. Overwritten variables must match exactly on a hit; for prepended
    /// ones only being set or not matters.
    pub inputs: BTreeMap<String, Option<String>>,
}

/// FNV-1a 64. Stable across Rust versions, unlike `DefaultHasher`.
pub fn fnv64(chunks: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for chunk in chunks {
        for b in *chunk {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // Separator, so ("ab", "c") and ("a", "bc") differ
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Key from the real manifest path, the contents of pixi.toml and pixi.lock, and
/// the pixi executable (path and mtime, so upgrading pixi invalidates).
pub fn key(manifest: &Path, pixi: &Path) -> Option<String> {
    let toml = std::fs::read(manifest).ok()?;
    let lock = std::fs::read(manifest.with_file_name("pixi.lock")).unwrap_or_default();
    let mtime = std::fs::metadata(pixi)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let h = fnv64(&[
        &VERSION.to_le_bytes(),
        manifest.as_os_str().as_encoded_bytes(),
        &toml,
        &lock,
        pixi.as_os_str().as_encoded_bytes(),
        &mtime.to_le_bytes(),
    ]);
    Some(format!("{h:016x}"))
}

pub fn lookup(file: &Path, key: &str, base: &EnvMap) -> Option<Activation> {
    let entry: Entry = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    if entry.version != VERSION || entry.key != key {
        return None;
    }
    if !entry.activation.prefix.join("conda-meta").is_dir() {
        return None;
    }
    let fits = entry.inputs.iter().all(|(k, recorded)| {
        let now = base.get(k);
        if entry.activation.prepend.contains_key(k) {
            now.is_some_and(|v| !v.is_empty()) == recorded.as_ref().is_some_and(|v| !v.is_empty())
        } else {
            now == recorded.as_ref()
        }
    });
    fits.then_some(entry.activation)
}

/// Best effort: a cache that cannot be written only costs speed.
pub fn store(
    file: &Path,
    key: &str,
    activation: &Activation,
    inputs: BTreeMap<String, Option<String>>,
) {
    let entry = Entry {
        version: VERSION,
        key: key.to_string(),
        activation: activation.clone(),
        inputs,
    };
    let Some(dir) = file.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = file.with_extension(format!("json.{}", std::process::id()));
    if let Ok(json) = serde_json::to_vec(&entry)
        && std::fs::write(&tmp, json).is_ok()
        && std::fs::rename(&tmp, file).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Prefix recorded in the cache, if any (pixi may put the env elsewhere).
pub fn cached_prefix(file: &Path) -> Option<std::path::PathBuf> {
    let entry: Entry = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    Some(entry.activation.prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        Activation,
    ) {
        let t = tempfile::tempdir().unwrap();
        let manifest = t.path().join("pixi.toml");
        fs::write(&manifest, "[workspace]\n").unwrap();
        fs::write(t.path().join("pixi.lock"), "version: 6\n").unwrap();
        let pixi = t.path().join("pixi");
        fs::write(&pixi, "").unwrap();
        let prefix = t.path().join(".pixi/envs/default");
        fs::create_dir_all(prefix.join("conda-meta")).unwrap();
        let act = Activation {
            prefix,
            prepend: [("PATH".into(), vec!["/p/bin".into()])].into(),
            set: [("CONDA_PREFIX".into(), "/p".into())].into(),
            unset: vec!["PYTHONHOME".into()],
        };
        (t, manifest, pixi, act)
    }

    #[test]
    fn key_changes_with_lock_and_separates_chunks() {
        let (t, manifest, pixi, _) = setup();
        let k1 = key(&manifest, &pixi).unwrap();
        assert_eq!(k1, key(&manifest, &pixi).unwrap());
        fs::write(t.path().join("pixi.lock"), "version: 6\nchanged\n").unwrap();
        assert_ne!(k1, key(&manifest, &pixi).unwrap());
        assert_ne!(fnv64(&[b"ab", b"c"]), fnv64(&[b"a", b"bc"]));
        assert!(key(&t.path().join("missing.toml"), &pixi).is_none());
    }

    #[test]
    fn lookup_checks_key_prefix_and_inputs() {
        let (t, _manifest, _pixi, act) = setup();
        let file = t.path().join("cache/pixi/a.json");
        let inputs = [
            ("PATH".to_string(), Some("/usr/bin".to_string())),
            ("CONDA_PREFIX".to_string(), None),
        ]
        .into();
        store(&file, "k", &act, inputs);

        let base: EnvMap = [("PATH".to_string(), "/other:/usr/bin".to_string())].into();
        assert_eq!(lookup(&file, "k", &base), Some(act.clone()));
        assert_eq!(lookup(&file, "other-key", &base), None);

        // CONDA_PREFIX was unset when pixi ran; now it is set, so the cached
        // CONDA_SHLVL etc. would be wrong
        let mut conda = base.clone();
        conda.insert("CONDA_PREFIX".into(), "/conda".into());
        assert_eq!(lookup(&file, "k", &conda), None);

        // Environment deleted
        fs::remove_dir_all(act.prefix.join("conda-meta")).unwrap();
        assert_eq!(lookup(&file, "k", &base), None);
    }
}
