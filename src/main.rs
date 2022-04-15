#![warn(rust_2018_idioms)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::unwrap_used)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::stdout;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use argh::FromArgs;
use cargo_metadata::diagnostic::{Diagnostic, DiagnosticCode};
use cargo_metadata::{CompilerMessage, Message};
use walkdir::WalkDir;

use clippy_lint_tester::clippy_workspace::{prepare_clippy, ClippyBin, ClippyWorkspace};
use clippy_lint_tester::markdown_formatting::print_table;
use clippy_lint_tester::{ensure_empty_dir, touch_crate_roots, EnsureEmptyDirOutcome, ProgressBar};

const CARGO_TARGET_DIR: &str = "_target";

#[derive(FromArgs)]
/// Test Clippy against downloaded crates
struct Args {
    #[argh(positional)]
    /// path to the Clippy source
    source: PathBuf,

    #[argh(positional)]
    /// path to the directory containing crates
    target: PathBuf,

    #[argh(positional)]
    /// lints to test
    lints: Vec<String>,

    #[argh(option)]
    /// the directory to attempt fixes in - omit to skip fixing
    fix: Option<PathBuf>,

    #[argh(switch)]
    /// check for allows - useful for testing attribute cleaning
    check_allows: bool,
}

fn crate_name(path: &Path) -> Cow<'_, str> {
    path.file_name().expect("has file_name").to_string_lossy()
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let Args {
        source,
        target,
        lints: lint_args,
        fix: fix_dir,
        check_allows,
    } = argh::from_env();

    for name in &lint_args {
        if name.is_empty()
            || name
                .strip_prefix("clippy::")
                .unwrap_or(name)
                .chars()
                .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        {
            bail!("Invalid lint name");
        }
    }

    if let Some(fix_dir) = &fix_dir {
        match ensure_empty_dir(fix_dir)? {
            EnsureEmptyDirOutcome::Created => println!("Fix directory created"),
            EnsureEmptyDirOutcome::NonEmpty => bail!("Fix directory exists and not empty"),
            EnsureEmptyDirOutcome::Empty => {}
        }
    }

    if !target.exists() {
        bail!("Target path `{}` does not exist", target.display())
    }

    let clippy_workspace = prepare_clippy(&env::current_dir()?.join(source), || {
        eprintln!("Compiling Clippy");
    })?;

    let lints = check_and_format_lint_names(&clippy_workspace, &lint_args)?;

    eprintln!("Linting crates");
    let mut paths = fs::read_dir(&target)
        .context("Failed to read target dir")?
        .map(|res| res.context("Failed to read entry").map(|e| e.path()))
        .filter(|res| {
            res.as_ref()
                .ok()
                .and_then(|p| p.file_name())
                .map_or(true, |n| n != CARGO_TARGET_DIR)
        })
        .collect::<Result<Vec<PathBuf>, anyhow::Error>>()?;
    paths.sort_unstable();

    let total_crates = paths.len();
    if total_crates == 0 {
        return Ok(());
    }

    let cargo_target_dir = env::current_dir()?.join(target).join(CARGO_TARGET_DIR);

    let mut build_failures = vec![];
    let mut fix_failures = vec![];

    let mut warning_counts = BTreeMap::new();
    let mut allow_counts: BTreeMap<Cow<'_, str>, _> = BTreeMap::new();

    {
        let mut progress_bar = ProgressBar::new();
        progress_bar.display_progress(total_crates, "Starting...");

        for path in &paths {
            let crate_name = crate_name(path);

            progress_bar.inc_progress(&crate_name);
            if check_allows && !lints.is_empty() {
                let count = check_for_allows(
                    &mut progress_bar,
                    &clippy_workspace,
                    &cargo_target_dir,
                    &lints,
                    path,
                    &crate_name,
                )?;
                if count > 0 {
                    allow_counts.insert(crate_name.clone(), count);
                }
            }

            let result = run_lint(
                &mut progress_bar,
                &clippy_workspace,
                &cargo_target_dir,
                &lints[..],
                path,
                fix_dir.as_deref(),
            )?;
            match result {
                LintResult::InvalidCrate => {
                    progress_bar.println(
                        &crate_name,
                        &format_args!("{} - not a crate", path.display()),
                    );
                }
                LintResult::BuildFailed => {
                    build_failures.push(crate_name);
                }
                LintResult::Success {
                    warning_count,
                    fix_failed,
                } => {
                    if warning_count > 0 {
                        if fix_failed {
                            fix_failures.push(crate_name.clone());
                        }
                        warning_counts.insert(crate_name, warning_count);
                    }
                }
            }
        }
    }

    println!();
    println!("# Summary");

    if !build_failures.is_empty() || lints.is_empty() {
        println!();
        println!("## Build failures");
        println!();
        println!("Total: {}", build_failures.len());
        if !build_failures.is_empty() {
            println!();
            for crate_name in &build_failures {
                println!("- {}", crate_name);
            }
        }
    }

    if !lints.is_empty() {
        println!();
        println!("## Warnings");
        println!();
        println!("Total: {}", warning_counts.values().sum::<usize>());
        if !warning_counts.is_empty() {
            println!();
            print_table(["Crate", "Count"], &warning_counts, stdout())?;
        }
    }

    if check_allows {
        println!();
        println!("## Allows");
        println!();
        println!("Total: {}", allow_counts.values().sum::<usize>());
        if !allow_counts.is_empty() {
            println!();
            print_table(["Crate", "Count"], &allow_counts, stdout())?;
        }
    }

    if fix_dir.is_some() {
        println!();
        println!("## Fix failures");
        println!();
        println!("Total: {}", fix_failures.len());

        if !fix_failures.is_empty() {
            println!();
            for crate_name in &fix_failures {
                println!("- {}", crate_name);
            }
        }
    }

    Ok(())
}

enum LintResult {
    InvalidCrate,
    BuildFailed,
    Success {
        warning_count: usize,
        fix_failed: bool,
    },
}

fn check_and_format_lint_names(
    clippy_workspace: &ClippyWorkspace,
    lint_args: &[String],
) -> Result<Vec<String>> {
    if lint_args.is_empty() {
        return Ok(vec![]);
    }

    eprintln!("Checking lint names");

    // Map formatted_name -> arg
    let mut formatted_names = BTreeMap::new();

    for lint_arg in lint_args {
        let mut formatted_name = lint_arg.to_lowercase();
        if !formatted_name.starts_with("clippy::") {
            formatted_name.insert_str(0, "clippy::");
        }
        formatted_name = formatted_name.replace('_', "-");

        formatted_names.insert(formatted_name, lint_arg);
    }

    let mut clippy_driver = clippy_workspace.make_clippy_command(ClippyBin::ClippyDriver);
    let output = clippy_driver
        .arg("-W")
        .arg("help")
        .output()
        .context("Running Clippy driver help")?;

    if !output.status.success() {
        bail!("Command to check lint names failed");
    }

    let mut lints = Vec::with_capacity(lint_args.len());
    let stdout = std::str::from_utf8(&output.stdout).context("Converting Cargo output to str")?;
    for help_lint in stdout
        .lines()
        .skip_while(|l| !l.starts_with("Lint checks provided by plugins"))
        .skip(1)
        .take_while(|l| !l.starts_with("Lint groups provided by plugins"))
        .filter_map(|l| l.split_whitespace().next())
    {
        if formatted_names.remove(help_lint).is_some() {
            lints.push(help_lint.replace('-', "_"));
        }

        if formatted_names.is_empty() {
            break;
        }
    }

    let mut missing_args = formatted_names.values();
    if let Some(first) = missing_args.next() {
        let mut error_message = format!("Lints not found: `{}`", first);
        for arg in missing_args {
            error_message.push_str(&format!(", `{}`", arg));
        }
        bail!(error_message);
    }

    Ok(lints)
}

fn make_lint_command(
    clippy_workspace: &ClippyWorkspace,
    cargo_target_dir: &Path,
    path: &Path,
) -> Command {
    let mut command = clippy_workspace.make_clippy_command(ClippyBin::CargoClippy);
    command
        .arg("--")
        .arg("--quiet")
        .arg("--message-format=json")
        .arg("--target-dir")
        .arg(cargo_target_dir)
        .arg("--")
        .arg("--cap-lints")
        .arg("warn")
        .arg("--allow")
        .arg("clippy::all")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(path);
    command
}

fn run_lint(
    progress_bar: &mut ProgressBar,
    clippy_workspace: &ClippyWorkspace,
    cargo_target_dir: &Path,
    lints: &[impl AsRef<str>],
    path: &Path,
    fix_dir: Option<&Path>,
) -> Result<LintResult> {
    let crate_name = crate_name(path);

    if !path.is_dir() || !path.join("Cargo.toml").exists() {
        return Ok(LintResult::InvalidCrate);
    }

    // Touch the crate roots to force recompilation.
    // Cargo can't detect changes to Clippy's source.
    touch_crate_roots(path).context("Touching crate roots")?;

    let mut cargo_clippy = make_lint_command(clippy_workspace, cargo_target_dir, path);
    for name in lints {
        cargo_clippy.arg("--warn").arg(name.as_ref());
    }

    let mut child = cargo_clippy.spawn().expect("command succeeds");

    let mut warning_count = 0;

    let reader = std::io::BufReader::new(child.stdout.take().expect("stdout piped"));
    for message in cargo_metadata::Message::parse_stream(reader) {
        if let Message::CompilerMessage(CompilerMessage {
            message:
                Diagnostic {
                    code: Some(DiagnosticCode { code, .. }),
                    spans,
                    rendered: Some(rendered),
                    ..
                },
            ..
        }) = message.context("parsing Cargo messages")?
        {
            if lints.iter().any(|name| code == name.as_ref()) {
                warning_count += 1;
                let span = &spans[0];
                progress_bar.println(&crate_name, "");
                progress_bar.println(
                    &crate_name,
                    &format_args!(
                        "---> {}/{}:{}:{}",
                        &crate_name, span.file_name, span.line_start, span.column_start
                    ),
                );
                progress_bar.println(&crate_name, &rendered.trim_end());
            }
        }
    }

    let status = child.wait().context("Waiting for Cargo command")?;

    if !status.success() {
        progress_bar.println(&crate_name, "");

        let mut errors = String::new();
        child
            .stderr
            .take()
            .expect("stderr piped")
            .read_to_string(&mut errors)
            .context("Reading stderr")?;

        let ice = errors.contains("internal compiler error: unexpected panic\n\nnote: the compiler unexpectedly panicked. this is a bug.");

        progress_bar.println(
            &crate_name,
            &format_args!(
                "{} - build failed{}",
                &crate_name,
                if ice { " (ICE)" } else { "" }
            ),
        );
        progress_bar.println(
            &crate_name,
            &format_args!("Command used: `{}`", format_command(&cargo_clippy)),
        );

        return Ok(LintResult::BuildFailed);
    }

    let mut fix_failed = false;
    if warning_count > 0 && !lints.is_empty() {
        if let Some(fix_dir) = fix_dir {
            let fix_dir = fix_dir.join(path.file_name().expect("Path not '..'"));
            copy_dir(progress_bar, path, &fix_dir)?;
            let fix_success = run_fix(
                progress_bar,
                clippy_workspace,
                cargo_target_dir,
                lints,
                &fix_dir,
                &crate_name,
            )?;
            if !fix_success {
                fix_failed = true;
            }
        }
    }

    Ok(LintResult::Success {
        warning_count,
        fix_failed,
    })
}

fn check_for_allows(
    progress_bar: &mut ProgressBar,
    clippy_workspace: &ClippyWorkspace,
    cargo_target_dir: &Path,
    lints: &[impl AsRef<str>],
    path: &Path,
    crate_name: &str,
) -> Result<usize> {
    let mut command = clippy_workspace.make_clippy_command(ClippyBin::CargoClippy);
    command
        .arg("--")
        .arg("--target-dir")
        .arg(cargo_target_dir)
        .arg("--quiet")
        .arg("--message-format=json")
        .arg("--")
        .arg("--allow")
        .arg("clippy::all");

    for name in lints {
        command.arg("--forbid").arg(name.as_ref());
    }

    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(path);

    let mut child = command.spawn().expect("command succeeds");
    let reader = std::io::BufReader::new(child.stdout.take().expect("stdout piped"));

    let mut count = 0;
    for message in cargo_metadata::Message::parse_stream(reader) {
        if let Message::CompilerMessage(CompilerMessage {
            message:
                Diagnostic {
                    code: Some(DiagnosticCode { code, .. }),
                    rendered: Some(rendered),
                    spans,
                    ..
                },
            ..
        }) = message.context("parsing Cargo messages")?
        {
            if code == "E0453" && spans.iter().all(|s| s.expansion.is_none()) {
                count += 1;
                let span = &spans[0];
                progress_bar.println(
                    crate_name,
                    &format_args!(
                        "---> {}/{}:{}:{}",
                        &crate_name, span.file_name, span.line_start, span.column_start
                    ),
                );
                progress_bar.println(crate_name, "Allow found");
                progress_bar.println(crate_name, &rendered);
            }
        }
    }

    Ok(count)
}

// Returns `true` if successful and `false` otherwise.
fn run_fix(
    progress_bar: &mut ProgressBar,
    clippy_workspace: &ClippyWorkspace,
    cargo_target_dir: &Path,
    lints: &[impl AsRef<str>],
    path: &Path,
    crate_name: &str,
) -> Result<bool> {
    let mut fix_command = clippy_workspace.make_clippy_command(ClippyBin::CargoClippy);

    fix_command
        .arg("--")
        .arg("--target-dir")
        .arg(cargo_target_dir)
        .arg("--fix")
        .arg("--broken-code")
        .arg("--allow-dirty")
        .arg("--allow-staged")
        .arg("--allow-no-vcs")
        .arg("--")
        .arg("--cap-lints")
        .arg("warn")
        .arg("--allow")
        .arg("clippy::all");

    for name in lints {
        fix_command.arg("--warn").arg(name.as_ref());
    }

    let fix_output = fix_command
        .current_dir(path)
        .output()
        .context("Executing fix command")?;

    let success = fix_output.status.success();

    if success {
        progress_bar.println(crate_name, &format_args!("{} - fix succeeded", &crate_name));
    } else {
        progress_bar.println(crate_name, &format_args!("{} - fix failed", &crate_name));
        let error =
            std::str::from_utf8(&fix_output.stderr).context("Converting Cargo output to str")?;
        progress_bar.println(crate_name, error);
    }

    Ok(success)
}

fn copy_dir(_progress_bar: &mut ProgressBar, clippy_source: &Path, target: &Path) -> Result<()> {
    for entry in WalkDir::new(clippy_source) {
        let entry = entry.with_context(|| format!("Reading {}", clippy_source.display()))?;
        let file_type = entry.file_type();

        let entry_target = target.join(
            entry
                .path()
                .strip_prefix(clippy_source)
                .expect("Entries of source"),
        );

        if file_type.is_dir() {
            fs::create_dir(&entry_target)
                .with_context(|| format!("Creating {}", &entry_target.display()))?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &entry_target)
                .with_context(|| format!("Copying {}", &entry_target.display()))?;
        }
    }

    Ok(())
}

fn format_command(command: &Command) -> String {
    let mut result = String::new();

    if let Some(current_dir) = command.get_current_dir() {
        result.push_str(&format!(
            "cd {} && ",
            &shell_escape::escape(current_dir.to_string_lossy()),
        ));
    }

    result.push_str(&shell_escape::escape(
        command.get_program().to_string_lossy(),
    ));

    for arg in command.get_args() {
        result.push_str(&format!(
            " {}",
            &shell_escape::escape(arg.to_string_lossy())
        ));
    }

    result
}
