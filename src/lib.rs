#![warn(rust_2018_idioms)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::unwrap_used)]
#![warn(clippy::print_stdout)]
#![warn(clippy::print_stderr)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use filetime::{set_file_mtime, FileTime};
use toml::map::Entry;
use toml::value::Table;
use toml::Value;
use walkdir::WalkDir;

pub mod attr_cleaning;
pub mod clippy_workspace;
pub mod markdown_formatting;
pub mod progress_bar;

use attr_cleaning::{clean_source, CleanError};

pub use progress_bar::ProgressBar;

pub enum EnsureEmptyDirOutcome {
    Created,
    Empty,
    NonEmpty,
}

pub fn ensure_empty_dir(path: &Path) -> Result<EnsureEmptyDirOutcome> {
    match path.read_dir() {
        Ok(mut dir) => Ok(if dir.next().is_none() {
            EnsureEmptyDirOutcome::Empty
        } else {
            EnsureEmptyDirOutcome::NonEmpty
        }),
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => {
                fs::create_dir_all(path)
                    .with_context(|| format!("Creating dir {}", path.display()))?;
                Ok(EnsureEmptyDirOutcome::Created)
            }
            _ => Err(err).context("Failed to read target"),
        },
    }
}

pub struct FileCleanError {
    pub path: PathBuf,
    pub error: CleanError,
}

// Remove all attrs from all source files that could affect linting.
pub fn clean_attrs(path: &Path) -> Result<Vec<FileCleanError>> {
    if path.is_file() {
        clean_attrs_file(path).map(|result| {
            result
                .map(|err| FileCleanError {
                    path: path.to_path_buf(),
                    error: err,
                })
                .into_iter()
                .collect()
        })
    } else if path.is_dir() {
        clean_attrs_dir(path)
    } else {
        bail!("Path not file or dir");
    }
}

// path must be for a dir
fn clean_attrs_dir(path: &Path) -> Result<Vec<FileCleanError>> {
    let mut errors = vec![];
    for entry in WalkDir::new(path) {
        let entry = entry.with_context(|| format!("Reading {}", path.display()))?;
        let file_type = entry.file_type();
        if file_type.is_file() && entry.path().extension().map_or(false, |e| e == "rs") {
            if let Ok(Some(err)) = clean_attrs_file(entry.path()) {
                errors.push(FileCleanError {
                    path: entry.path().to_path_buf(),
                    error: err,
                });
            }
        }
    }
    Ok(errors)
}

// path must be for a file
fn clean_attrs_file(path: &Path) -> Result<Option<CleanError>> {
    let source =
        fs::read_to_string(&path).with_context(|| format!("Reading file {}", path.display()))?;
    match clean_source(&source) {
        Ok(None) => Ok(None),
        Ok(Some(cleaned)) => {
            let backup = backup_path(path);
            fs::copy(&path, &backup)
                .with_context(|| format!("Copying {} to {}", path.display(), backup.display()))?;
            fs::write(&path, cleaned).with_context(|| format!("Writing to {}", path.display()))?;
            Ok(None)
        }
        Err(err) => Ok(Some(err)),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut ext = path.extension().unwrap_or_else(|| "".as_ref()).to_owned();
    ext.push(".orig");
    path.with_extension(ext)
}

pub fn clean_config(path: &Path) -> Result<()> {
    let manifest_path = path.join("Cargo.toml");
    clean_cargo_manifest(&manifest_path)?;

    disable_clippy_config(path)?;

    Ok(())
}

pub fn touch_crate_roots(crate_path: &Path) -> Result<()> {
    let manifest_path = crate_path.join("Cargo.toml");

    let contents = fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read Cargo.toml '{}'", crate_path.display()))?;
    let mut root: Value = contents
        .parse()
        .with_context(|| format!("Failed to parse Cargo.toml '{}'", crate_path.display()))?;

    if let Value::Table(root_table) = &mut root {
        if let Some(Value::Table(section)) = root_table.get("lib") {
            if let Some(Value::String(path)) = section.get("path") {
                let root_path = crate_path.join(path);
                set_file_mtime(&root_path, FileTime::now()).with_context(|| {
                    format!("Failed to set mtime for '{}'", root_path.display())
                })?;
            }
        }

        if let Some(Value::Array(sections)) = root_table.get("bin") {
            for section in sections {
                if let Some(Value::String(path)) = section.get("path") {
                    let root_path = crate_path.join(path);
                    set_file_mtime(&root_path, FileTime::now()).with_context(|| {
                        format!("Failed to set mtime for '{}'", root_path.display())
                    })?;
                }
            }
        }
    }

    for default_root in ["src/lib.rs", "src/main.rs"] {
        let path = crate_path.join(default_root);
        match set_file_mtime(&path, FileTime::now()) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("Failed to set mtime for '{}'", path.display()))
            }
        };
    }

    Ok(())
}

// Replace path dependencies with crate versions.
fn clean_cargo_manifest(path: &Path) -> Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read Cargo.toml '{}'", path.display()))?;
    let mut root: Value = contents
        .parse()
        .with_context(|| format!("Failed to parse Cargo.toml '{}'", path.display()))?;

    let mut paths_removed = false;
    if let Value::Table(root_table) = &mut root {
        paths_removed = remove_paths(root_table, "dependencies")
            | remove_paths(root_table, "build-dependencies")
            | remove_paths(root_table, "dev-dependencies")
            | root_table.remove("workspace").is_some();
    }

    if paths_removed {
        let backup_path = path.with_extension("toml.bak");
        fs::copy(path, &backup_path)
            .with_context(|| format!("Making Cargo.toml backup '{}'", &backup_path.display()))?;
        fs::write(path, root.to_string())
            .with_context(|| format!("Replace Cargo.toml contents '{}'", path.display()))?;
    }

    Ok(())
}

fn remove_paths(root_table: &mut Table, name: &str) -> bool {
    let mut result = false;
    if let Some(Value::Table(dep_table)) = root_table.get_mut(name) {
        for (_, locations) in dep_table.iter_mut() {
            if let Value::Table(loc_table) = locations {
                let removed_path = loc_table.remove("path").is_some();
                if removed_path {
                    if let Entry::Vacant(entry) = loc_table.entry("version") {
                        entry.insert(Value::String("*".into()));
                    }
                }
                result |= removed_path;
            }
        }
    }
    result
}

fn disable_clippy_config(path: &Path) -> Result<()> {
    for name in &[".clippy.toml", "clippy.toml"] {
        let config_path = path.join(name);
        if config_path.exists() {
            fs::rename(&config_path, config_path.with_extension("toml.bak"))
                .with_context(|| format!("Renaming {}", config_path.display()))?;
        }
    }

    Ok(())
}
