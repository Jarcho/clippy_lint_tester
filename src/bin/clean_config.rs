use std::path::PathBuf;

use anyhow::Result;
use argh::FromArgs;
use clippy_lint_tester::clean_config;

#[derive(FromArgs)]
/// Modify the Cargo manifest and Clippy config for testing
struct Args {
    #[argh(positional)]
    target: PathBuf,
}

fn main() -> Result<()> {
    let Args { target } = argh::from_env();

    clean_config(&target)?;

    Ok(())
}
