//! Messages for humans. They always go to stderr: in `--emit` mode stdout is
//! shell code that gets evaluated, and elsewhere it is data (`list`, `path`).

use console::{Style, style};

pub fn ok(s: impl AsRef<str>) {
    eprintln!("{} {}", style("✔").green().for_stderr(), s.as_ref());
}

pub fn warn(s: impl AsRef<str>) {
    eprintln!("{} {}", style("!").yellow().for_stderr(), s.as_ref());
}

pub fn fail(s: impl AsRef<str>) {
    eprintln!("{} {}", style("✘").red().bold().for_stderr(), s.as_ref());
}

pub fn info(s: impl AsRef<str>) {
    eprintln!("{} {}", style("→").cyan().for_stderr(), s.as_ref());
}

/// Indented follow-up line, e.g. what to run next.
pub fn hint(s: impl AsRef<str>) {
    eprintln!("  {}", s.as_ref());
}

pub fn dim(s: impl AsRef<str>) -> String {
    Style::new()
        .dim()
        .for_stderr()
        .apply_to(s.as_ref())
        .to_string()
}

pub fn bold(s: impl AsRef<str>) -> String {
    Style::new()
        .bold()
        .for_stderr()
        .apply_to(s.as_ref())
        .to_string()
}

/// Pad to a display width (CJK characters count as two columns).
pub fn pad(s: &str, width: usize) -> String {
    let w = console::measure_text_width(s);
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}
