#![warn(rust_2018_idioms)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use argh::FromArgs;
use flate2::read::GzDecoder;
use serde::Deserialize;
use tar::Archive;
use ureq::{Agent, AgentBuilder};

use clippy_lint_tester::{
    clean_attrs, clean_config, ensure_empty_dir, EnsureEmptyDirOutcome, FileCleanError, ProgressBar,
};

#[derive(FromArgs)]
/// Download the all-time most downloaded crates on crates.io and remove any settings
/// (lint attributes, clippy.config, etc.) that may interfere with lint testing.
/// Removing lint attributes is 'best effort'. Use `--show-attr-errors` to display errors.
struct Args {
    #[argh(positional)]
    target: PathBuf,
    /// the number of crates to download
    #[argh(option, short = 'n', default = "50")]
    number: usize,
    /// crates to exclude
    #[argh(option, short = 'x')]
    exclude: Vec<String>,
    /// display attribute removal errors
    #[argh(switch)]
    show_attr_errors: bool,
}

#[derive(Deserialize, Debug)]
struct CratePage {
    crates: Vec<Crate>,
}

#[derive(Deserialize, Debug)]
struct Crate {
    name: String,
    max_version: String,
    max_stable_version: Option<String>,
}

impl Crate {
    fn version(&self) -> &str {
        self.max_stable_version
            .as_ref()
            .unwrap_or(&self.max_version)
    }
}

const CRATES_IO_MAX_PER_PAGE: usize = 100;

fn main() -> Result<()> {
    let Args {
        target,
        number,
        exclude,
        show_attr_errors,
    } = argh::from_env();

    if number == 0 {
        bail!("The number of crates must be positive.")
    }

    match ensure_empty_dir(&target)? {
        EnsureEmptyDirOutcome::Created => println!("Target directory created"),
        EnsureEmptyDirOutcome::NonEmpty => bail!("Target exists and not empty"),
        EnsureEmptyDirOutcome::Empty => {}
    }

    match target.read_dir() {
        Ok(mut dir) => {
            if dir.next().is_some() {
                bail!("Target dir exists and is not empty")
            }
        }
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => {
                fs::create_dir_all(&target).context("Failed to create target")?;
            }
            _ => return Err(err).context("Failed to read target"),
        },
    }

    let mut progress_bar = ProgressBar::new();
    progress_bar.display_progress(number, "Starting...");

    let mut agent: Agent = AgentBuilder::new().build();

    let mut downloaded_crates = BTreeSet::new();
    for krate in list_crates(&exclude).take(number) {
        let krate = krate?;
        let crate_path = &target.join(format!("{}-{}", &krate.name, &krate.version()));

        progress_bar.inc_progress(&krate.name);
        if downloaded_crates.contains(&krate.name) {
            progress_bar.println(
                &krate.name,
                &format!(
                    "Skipping '{}'. Listed twice by crates.io. (Possibly the changed position during downloading.)",
                    &krate.name
                ),
            );
            continue;
        }
        download_crate(&mut agent, &krate, &target)?;
        clean_config(crate_path)?;

        let errors = clean_attrs(crate_path)?;
        if show_attr_errors {
            for FileCleanError { path, error } in errors {
                progress_bar.println(
                    &krate.name,
                    &format!(
                        "error: Attribute removal failed at {}:{}:{} - {}",
                        path.display(),
                        error.line,
                        error.column,
                        error.message,
                    ),
                );
            }
        }

        remove_cargo_config(crate_path)?;

        downloaded_crates.insert(krate.name);
    }

    Ok(())
}

fn list_crates(exclude: &[String]) -> impl Iterator<Item = Result<Crate>> + '_ {
    // We're using crates.io API.
    // We need to conform to https://crates.io/policies#crawlers.

    // The crawler policy requires a limit of one request per second.
    // We're always slower than this because we leave at least a second between requests.
    const MIN_TIME_BETWEEN: Duration = Duration::from_secs(1);

    let agent = AgentBuilder::new()
        // User agent required by crawler policy.
        .user_agent("clippy_lint_tester (mikerite@lavabit.com)")
        .build();

    let mut last_request_time = None;

    (1..)
        .map(move |page_num| {
            let now = Instant::now();
            if let Some(last) = last_request_time {
                let time_between = now.duration_since(last);
                if let Some(sleep_dur) = MIN_TIME_BETWEEN.checked_sub(time_between) {
                    std::thread::sleep(sleep_dur);
                }
            }
            let url = format!(
                "https://crates.io/api/v1/crates?page={}&per_page={}&sort=downloads",
                page_num, CRATES_IO_MAX_PER_PAGE,
            );
            let response = agent.get(&url).call().context("Failed to get crate page");
            last_request_time = Some(Instant::now());
            (url, response)
        })
        .map(|(url, response)| {
            response.and_then(|r| {
                r.into_json()
                    .with_context(|| format!("Failed to parse crate page: {}", url))
            })
        })
        .flat_map(move |page: Result<CratePage>| {
            let (ok, err) = match page {
                Ok(page) => (Some(page), None),
                Err(err) => (None, Some(err)),
            };
            ok.into_iter()
                .flat_map(move |page| {
                    page.crates
                        .into_iter()
                        .filter(|c| c.max_version != "0.0.0") // Skip yanked crates
                        .filter(move |c| !exclude.contains(&c.name))
                        .map(Result::Ok)
                })
                .chain(err.into_iter().map(Result::Err))
        })
}

fn download_crate(agent: &mut Agent, krate: &Crate, path: &Path) -> Result<()> {
    let reader = agent
        .get(&format!(
            "https://static.crates.io/crates/{name}/{name}-{version}.crate",
            name = krate.name,
            version = krate.version(),
        ))
        .call()
        .with_context(|| format!("Failed to download crate '{}'", krate.name))?
        .into_reader();

    let decoder = GzDecoder::new(reader);

    let mut archive = Archive::new(decoder);
    archive.set_overwrite(false);
    archive
        .unpack(path)
        .with_context(|| format!("Failed to unpack crate '{}'", krate.name))
}

fn remove_cargo_config(crate_path: &Path) -> Result<()> {
    match fs::remove_file(crate_path.join(".cargo").join("config")) {
        Ok(()) => Ok(()),
        Err(err) => match err.kind() {
            io::ErrorKind::NotFound => Ok(()),
            _ => Err(err).context("Failed to delete config file"),
        },
    }
}
