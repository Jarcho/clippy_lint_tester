use std::path::PathBuf;

use anyhow::Result;
use argh::FromArgs;
use clippy_lint_tester::clean_attrs;

#[derive(FromArgs)]
/// Remove all attrs that might affect linting.
struct Args {
    #[argh(positional)]
    /// path to the file or dir to clean
    path: PathBuf,
}

fn main() -> Result<()> {
    let Args { path } = argh::from_env();

    clean_attrs(&path)?;

    Ok(())
}
