#![warn(rust_2018_idioms)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]

use expect_test::expect;
use regex::Regex;
use tempfile::tempdir;

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::str;

// Random string unlikely to exist as a path or file name
const NON_EXISTING: &str = "56427a04-e414-4ca3-880c-af2b58bf0492";

struct CommandOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

#[derive(PartialEq, Eq)]
enum ClippyWorkspace<'a> {
    Default,
    Named(&'a str),
    Custom(&'a str),
    NonExisting,
}

impl<'a> ClippyWorkspace<'a> {
    fn to_arg(&'a self) -> Cow<'a, OsStr> {
        let name = match *self {
            ClippyWorkspace::Default => "default",
            ClippyWorkspace::Named(name) => name,
            ClippyWorkspace::Custom(custom) => return OsStr::new(custom).into(),
            ClippyWorkspace::NonExisting => return OsStr::new(NON_EXISTING).into(),
        };

        let mut outcome = test_dir();
        outcome.push("clippy_workspaces");
        outcome.push(name);
        assert!(
            outcome.exists(),
            "{}",
            if *self == ClippyWorkspace::Default {
                "A clippy workspace at tests/rust-clippy is required for testing. Run \
                `git clone -b rust-1.59.0 --depth 1 https://github.com/rust-lang/rust-clippy default` \
                in the tests/clippy_workspaces directory to set one up."
            } else {
                "Workspace not found. Use `ClippyWorkspace::NonExisting` to test missing workspaces."
            }
        );
        outcome.into_os_string().into()
    }
}

enum TargetDir<'a> {
    Default,
    Named(&'a str),
    Custom(&'a str),
    NonExisting,
}

impl<'a> TargetDir<'a> {
    fn to_arg(&'a self) -> Cow<'a, OsStr> {
        let name = match *self {
            TargetDir::Default => "default",
            TargetDir::Named(name) => name,
            TargetDir::Custom(custom) => return OsStr::new(custom).into(),
            TargetDir::NonExisting => return OsStr::new(NON_EXISTING).into(),
        };

        let mut outcome = test_dir();
        outcome.push("targets");
        outcome.push(name);
        outcome.into_os_string().into()
    }
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).into()
}

fn test_dir() -> PathBuf {
    project_root().join("tests")
}

#[derive(PartialEq, Eq)]
enum TesterOption<'a> {
    CheckAllows,
    Fix(&'a OsStr),
}

fn run_clippy_lint_tester(
    clippy_workspace: &ClippyWorkspace<'_>,
    target_dir: &TargetDir<'_>,
    lints: &[&str],
    options: &[TesterOption<'_>],
) -> CommandOutput {
    fn clean(stream: Vec<u8>) -> String {
        String::from_utf8(stream)
            .expect("utf8 stdout")
            .replace(test_dir().to_str().unwrap(), "TEST_DIR")
            .replace(project_root().to_str().unwrap(), "PROJ_ROOT")
            .replace(NON_EXISTING, "NON_EXISTING")
    }

    let exe = Path::new(env!("CARGO_BIN_EXE_clippy_lint_tester"));
    let mut command = Command::new(exe);
    command
        .arg(&clippy_workspace.to_arg())
        .arg(&target_dir.to_arg())
        .args(lints);

    for option in options {
        match option {
            TesterOption::CheckAllows => {
                command.arg("--check-allows");
            }
            TesterOption::Fix(fix_dir) => {
                command.arg("--fix").arg(fix_dir);
            }
        }
        if *option == TesterOption::CheckAllows {}
    }

    let output = command.output().expect("Command succeeds");

    CommandOutput {
        status: output.status,
        stdout: clean(output.stdout),
        stderr: clean(output.stderr),
    }
}

#[test]
fn success() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Default,
        &["approx_constant"],
        &[],
    );

    let expected_stdout = expect![[r###"

        ---> a/src/main.rs:2:14
        warning: approximate value of `f{32, 64}::consts::PI` found
         --> src/main.rs:2:14
          |
        2 |     let pi = 3.14;
          |              ^^^^
          |
          = note: requested on the command line with `-W clippy::approx-constant`
          = help: consider using the constant directly
          = help: for further information visit https://rust-lang.github.io/rust-clippy/master/index.html#approx_constant

        # Summary

        ## Warnings

        Total: 1

         Crate | Count 
        :------|------:
         a     |     1 
    "###]];

    let expected_stderr = expect![[r#"
        Compiling Clippy
        Checking lint names
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert!(output.status.success());
}

#[test]
fn clippy_workspace_build_failure() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Named("build_failure"),
        &[],
        &[],
    );
    let expected_stdout = expect![[r###"

        a - build failed
        Command used: `cd TEST_DIR/targets/build_failure/a && cargo +nightly-2021-12-30 --quiet run --manifest-path=TEST_DIR/clippy_workspaces/default/Cargo.toml --release --bin cargo-clippy -- -- --quiet --message-format=json --target-dir TEST_DIR/targets/build_failure/_target -- --cap-lints warn --allow 'clippy::all'`

        # Summary

        ## Build failures

        Total: 1

        - a
    "###]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn lints_invalid() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Default,
        &["approx_constant", "bad_lint_1", "bad_lint_2"],
        &[],
    );

    let expected_stdout = expect![[r#""#]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Checking lint names
        Error: Lints not found: `bad_lint_1`, `bad_lint_2`
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn lint_groups_not_supported() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Default,
        &["correctness"],
        &[],
    );

    let expected_stdout = expect![[r#""#]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Checking lint names
        Error: Lints not found: `correctness`
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn target_has_non_crates() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Named("non_crates"),
        &[],
        &[],
    );

    let expected_stdout = expect![[r###"
        TEST_DIR/targets/non_crates/a.txt - not a crate
        TEST_DIR/targets/non_crates/b - not a crate

        # Summary

        ## Build failures

        Total: 0
    "###]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn clippy_workspace_has_missing_toolchain() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Named("missing_toolchain"),
        &TargetDir::Default,
        &[],
        &[],
    );

    let expected_stdout = expect![[r#""#]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Error: Failed to build Clippy
        stderr: error: toolchain 'TOOLCHAIN' is not installed

    "#]];

    let toolchain_regex = Regex::new(r"'[^']*'").unwrap();

    expected_stderr.assert_eq(&toolchain_regex.replace(&output.stderr, "'TOOLCHAIN'"));
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn clippy_workspace_does_not_exist() {
    let output =
        run_clippy_lint_tester(&ClippyWorkspace::NonExisting, &TargetDir::Default, &[], &[]);

    let expected_stdout = expect![[r#""#]];
    let expected_stderr = expect![[r#"
        Error: Source path `PROJ_ROOT/NON_EXISTING` does not exist
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn clippy_workspace_relative_path() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Custom("tests/clippy_workspaces/default"),
        &TargetDir::Default,
        &[],
        &[],
    );

    let expected_stdout = expect![[r###"

        # Summary

        ## Build failures

        Total: 0
    "###]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn target_relative_path() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Custom("tests/targets/default"),
        &[],
        &[],
    );

    let expected_stdout = expect![[r###"

        # Summary

        ## Build failures

        Total: 0
    "###]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn target_does_not_exist() {
    let output =
        run_clippy_lint_tester(&ClippyWorkspace::Default, &TargetDir::NonExisting, &[], &[]);
    let expected_stdout = expect![[r#""#]];
    let expected_stderr = expect![[r#"
        Error: Target path `NON_EXISTING` does not exist
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn check_allows() {
    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Named("check_allows"),
        &["approx_constant"],
        &[TesterOption::CheckAllows],
    );
    let expected_stdout = expect![[r###"
        ---> a/src/main.rs:1:10
        Allow found
        error[E0453]: allow(clippy::approx_constant) incompatible with previous forbid
         --> src/main.rs:1:10
          |
        1 | #![allow(clippy::approx_constant)]
          |          ^^^^^^^^^^^^^^^^^^^^^^^ overruled by previous forbid
          |
          = note: `forbid` lint level was set on command line



        # Summary

        ## Warnings

        Total: 0

        ## Allows

        Total: 1

         Crate | Count 
        :------|------:
         a     |     1 
    "###]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Checking lint names
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn test_fix() {
    let fix_dir = tempdir().unwrap();

    let output = run_clippy_lint_tester(
        &ClippyWorkspace::Default,
        &TargetDir::Named("fix"),
        &["needless_return"],
        &[TesterOption::Fix(fix_dir.path().as_os_str())],
    );
    let expected_stdout = expect![[r###"

        ---> a/src/main.rs:6:5
        warning: unneeded `return` statement
         --> src/main.rs:6:5
          |
        6 |     return "Hello, world!";
          |     ^^^^^^^^^^^^^^^^^^^^^^^ help: remove `return`: `"Hello, world!"`
          |
          = note: requested on the command line with `-W clippy::needless-return`
          = help: for further information visit https://rust-lang.github.io/rust-clippy/master/index.html#needless_return
        a - fix succeeded

        # Summary

        ## Warnings

        Total: 1

         Crate | Count 
        :------|------:
         a     |     1 

        ## Fix failures

        Total: 0
    "###]];
    let expected_stderr = expect![[r#"
        Compiling Clippy
        Checking lint names
        Linting crates
    "#]];

    expected_stderr.assert_eq(&output.stderr);
    expected_stdout.assert_eq(&output.stdout);
    assert_eq!(output.status.code(), Some(0));

    let fixed_file =
        fs::read_to_string(fix_dir.path().join("a/src/main.rs")).expect("fixed file exists");

    let expected_fixed_file = expect![[r#"
        fn main() {
            println!("{}", foo());
        }

        fn foo() -> &'static str {
            "Hello, world!"
        }
    "#]];
    expected_fixed_file.assert_eq(&fixed_file);

    fix_dir.close().unwrap();
}
