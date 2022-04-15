use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RustToolchainFile {
    toolchain: RustToolchain,
}

#[derive(Debug, Deserialize)]
struct RustToolchain {
    channel: String,
}

pub struct ClippyWorkspace {
    // The toolchain arg (e.g. +nightly-2021-03-25)
    toolchain_arg: OsString,
    // The manifest arg (e.g. --manifest-path=/home/mike/projects/rust-clippy/Cargo.toml)
    manifest_arg: OsString,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ClippyBin {
    CargoClippy,
    ClippyDriver,
}

// Builds clippy in release mode and ensure that it works.
pub fn prepare_clippy(
    clippy_source: &Path,
    pre_compile_callback: impl Fn(),
) -> Result<ClippyWorkspace> {
    assert!(
        clippy_source.is_absolute(),
        "`clippy_source` must be absolute"
    );

    if !clippy_source.exists() {
        bail!("Source path `{}` does not exist", clippy_source.display())
    }

    let toolchain_path = clippy_source.join("rust-toolchain");
    let toolchain_contents = match fs::read_to_string(toolchain_path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            bail!(
                "`{}` is not a Clippy workspace directory",
                clippy_source.display()
            )
        }
        Err(err) => {
            return Err(anyhow::Error::new(err))
                .context("Failed to read Clippy rust-toolchain file")
        }
    };

    let toolchain_file: RustToolchainFile =
        toml::from_str(&toolchain_contents).context("Parsing rust-toolchain toml")?;

    let mut toolchain_arg: OsString = "+".into();
    toolchain_arg.push(toolchain_file.toolchain.channel);

    let mut manifest_arg: OsString = "--manifest-path=".into();
    manifest_arg.push(clippy_source.join("Cargo.toml"));

    pre_compile_callback();

    let output = Command::new("cargo")
        .arg(&toolchain_arg)
        .arg("build")
        .arg(&manifest_arg)
        .arg("--release")
        .output()
        .expect("command succeeds");

    if !output.status.success() {
        bail!(
            "Failed to build Clippy\nstderr: {}",
            std::str::from_utf8(&output.stderr).context("Converting Cargo output to str")?
        );
    }

    Ok(ClippyWorkspace {
        toolchain_arg,
        manifest_arg,
    })
}

impl ClippyWorkspace {
    #[must_use]
    pub fn make_clippy_command(&self, bin: ClippyBin) -> Command {
        let mut command = Command::new("cargo");
        let cargo_run_args: &[&OsStr] = &[
            &self.toolchain_arg,
            "--quiet".as_ref(),
            "run".as_ref(),
            &self.manifest_arg,
            "--release".as_ref(),
            "--bin".as_ref(),
            match bin {
                ClippyBin::CargoClippy => "cargo-clippy",
                ClippyBin::ClippyDriver => "clippy-driver",
            }
            .as_ref(),
            "--".as_ref(), // end cargo run args
        ];
        command.args(cargo_run_args);
        command
    }
}
