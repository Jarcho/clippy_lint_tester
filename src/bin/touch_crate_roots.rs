use std::path::PathBuf;

use anyhow::Result;
use argh::FromArgs;
use clippy_lint_tester::touch_crate_roots;

#[derive(FromArgs)]
/// Touch crate roots to force recompilation
struct Args {
    #[argh(positional)]
    target: PathBuf,
}

fn main() -> Result<()> {
    let Args { target } = argh::from_env();

    touch_crate_roots(&target)?;

    Ok(())
}
