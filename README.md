# Clippy Lint Tester

Clippy Lint Tester is a utility that automates testing Clippy lints. Provided with a
Clppy workspace and directory of crates, it will lint each one and report on the results.

## SECURITY WARNING

Cargo will run arbitrary code provided by a crate while compiling it. Never use this application
on untrusted crates. This application provides no sandboxing whatsoever.

## Typical Workflow

``` bash

# Download the top 50 crates from crates.io
cargo run --bin download_crates crates

# Run clippy on the crates without linting anything
cargo run -- ~/projects/rust-clippy crates/

# Remove crates that don't build
# ...

# Check your_lint for false positives, etc.
cargo run -- ~/projects/rust-clippy crates/ your_lint
```

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
