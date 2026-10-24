//! Confirmation before deleting. `--yes` skips it; without a terminal it is an
//! error rather than a silent yes.

use std::fmt;
use std::io::IsTerminal;

use anyhow::{Result, bail};
use console::Term;
use dialoguer::Confirm;
use dialoguer::theme::ColorfulTheme;

/// Returned (wrapped in anyhow) when the user declines or hits Ctrl-C.
#[derive(Debug)]
pub struct Aborted;

impl fmt::Display for Aborted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("已取消")
    }
}

impl std::error::Error for Aborted {}

pub fn confirm(question: &str, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        bail!("需要确认，但当前不是交互终端；确定要这样做请加 --yes");
    }
    let answer = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(question)
        .default(false)
        .interact_on_opt(&Term::stderr())?;
    match answer {
        Some(true) => Ok(()),
        _ => Err(Aborted.into()),
    }
}
