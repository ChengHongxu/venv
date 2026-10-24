//! Edits to a python-venv environment's `requirements.txt`. Only the lines for
//! the packages named on the command line change: comments, `-r`/`-e` lines and
//! hand-written constraints stay. `pip freeze` would pull in every transitive
//! dependency, so it is not used.

/// PEP 503 normalized name: lowercase, runs of `-_.` become one `-`.
pub fn normalize(name: &str) -> String {
    let mut out = String::new();
    let mut sep = false;
    for c in name.chars() {
        if "-_.".contains(c) {
            sep = true;
        } else {
            if sep && !out.is_empty() {
                out.push('-');
            }
            sep = false;
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// Normalized project name of a requirement line or spec (`numpy>=2`,
/// `requests[socks]==2.32`). `None` for comments, options and paths/URLs.
pub fn name_of(line: &str) -> Option<String> {
    let line = line.split(" #").next()?.trim();
    if line.is_empty() || line.starts_with(['#', '-', '.', '/']) || line.contains("://") {
        return None;
    }
    let end = line
        .find(|c: char| !(c.is_ascii_alphanumeric() || "-_.".contains(c)))
        .unwrap_or(line.len());
    let name = normalize(&line[..end]);
    (!name.is_empty()).then_some(name)
}

/// Add each spec: replace the line for the same package, or append.
pub fn add(content: &str, specs: &[String]) -> String {
    let mut lines: Vec<String> = content.lines().map(str::to_string).collect();
    for spec in specs {
        match name_of(spec) {
            Some(name) => match lines
                .iter()
                .position(|l| name_of(l).as_ref() == Some(&name))
            {
                Some(i) => lines[i] = spec.clone(),
                None => lines.push(spec.clone()),
            },
            None if !lines.iter().any(|l| l.trim() == spec) => lines.push(spec.clone()),
            None => {}
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Remove the lines for `names`. Also returns the names that had no line.
pub fn remove(content: &str, names: &[String]) -> (String, Vec<String>) {
    let wanted: Vec<String> = names.iter().filter_map(|n| name_of(n)).collect();
    let mut found = vec![false; wanted.len()];
    let kept: Vec<&str> = content
        .lines()
        .filter(
            |l| match name_of(l).and_then(|n| wanted.iter().position(|w| *w == n)) {
                Some(i) => {
                    found[i] = true;
                    false
                }
                None => true,
            },
        )
        .collect();
    let missing = names
        .iter()
        .zip(&found)
        .filter(|(_, f)| !**f)
        .map(|(n, _)| n.clone())
        .collect();
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    (out, missing)
}

/// Whether there is anything for `pip install -r` to do.
pub fn has_requirements(content: &str) -> bool {
    content.lines().any(|l| {
        let l = l.trim();
        !l.is_empty() && !l.starts_with('#')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn names() {
        assert_eq!(normalize("Foo__Bar.baz"), "foo-bar-baz");
        assert_eq!(name_of("numpy>=2").as_deref(), Some("numpy"));
        assert_eq!(
            name_of("Requests[socks]==2.32 # pinned").as_deref(),
            Some("requests")
        );
        assert_eq!(
            name_of("python_dateutil ; python_version>'3'").as_deref(),
            Some("python-dateutil")
        );
        for none in ["", "# c", "-r base.txt", "-e .", "./pkg", "https://x/y.whl"] {
            assert_eq!(name_of(none), None, "{none}");
        }
    }

    #[test]
    fn add_replaces_or_appends() {
        let content = "# my deps\n-r base.txt\nnumpy>=1.26\nrequests\n";
        let out = add(content, &s(&["NumPy>=2", "click"]));
        assert_eq!(out, "# my deps\n-r base.txt\nNumPy>=2\nrequests\nclick\n");
        assert_eq!(add("", &s(&["six"])), "six\n");
        assert_eq!(add("six", &s(&["six"])), "six\n");
    }

    #[test]
    fn remove_keeps_other_lines() {
        let content = "# my deps\n-r base.txt\nnumpy>=2\nRequests[socks]\n";
        let (out, missing) = remove(content, &s(&["requests", "click"]));
        assert_eq!(out, "# my deps\n-r base.txt\nnumpy>=2\n");
        assert_eq!(missing, ["click"]);
        assert!(!has_requirements("# only comments\n\n"));
        assert!(has_requirements("# c\nsix\n"));
    }
}
