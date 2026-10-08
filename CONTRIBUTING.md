# Contributing to iris

## Toolchain

`rust-toolchain.toml` pins Rust 1.98.0 with `clippy` and `rustfmt`. Install [rustup](https://rustup.rs), then install the pinned toolchain from the checkout; `cargo` in the checkout uses it. CI builds and tests with the same toolchain.

```sh
rustup toolchain install
```

[Building](docs/building.md) lists the system packages each platform needs.

## Build

```sh
cargo build --release --bin iris
```

The executable is `target/release/iris` (`target\release\iris.exe` on Windows), unless the Cargo configuration sets another target directory.

## Test

```sh
cargo test --locked
```

The recorder tests run `ffmpeg` and `ffprobe` from `PATH`. On Linux, the window tests run only when `IRIS_X11_TEST_DISPLAY` is set to an X display; run them on a private Xvfb server:

```sh
xvfb-run -a -s '-screen 0 1600x1000x24' \
  sh -c 'IRIS_X11_TEST_DISPLAY=$DISPLAY cargo test --locked'
```

[Building](docs/building.md#tests) lists the other test requirements.

A change to observable behavior includes a test that fails without the change.

## Lint

CI runs these on every push to `main` and every pull request. Compiler and clippy warnings are errors:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo audit
```

`cargo audit` comes from `cargo install cargo-audit`. `clippy.toml` disallows calls that bypass iris's window, scroll, and executable-path helpers; the message of each finding states the helper to call instead.

Run `shellcheck` on every shell script you change:

```sh
shellcheck scripts/release-prep.sh
```

## Commits

- Write the subject line in the imperative mood, state the change, and end it without a period: `Watch the capture folders in place of the library's 1.5 s store poll`.
- Prefix a dependency update with `deps:` and a CI change with `ci:`.
- In the body, state the cause of a fix and the measurements behind a performance change. Wrap it at 72 columns.
- Keep each commit to one change, with its tests and documentation in the same commit.
- Add one sentence per user-visible change under `## [Unreleased]` in `CHANGELOG.md`, in the `### Added`, `### Changed`, `### Fixed`, `### Removed`, or `### Security` subsection.

Open pull requests against `main`. CI runs the tests on Linux, macOS, and Windows, runs the checks above, and builds every package.

## Documentation

The manual is in `docs/`, starting at [docs/SUMMARY.md](docs/SUMMARY.md). A change to behavior updates the chapter that describes that behavior in the same commit.

- Write flat declarative sentences in the present tense, and the imperative for instructions.
- State facts. Leave out hype, filler words, and sentences about the document itself.
- Do not use em dashes.
- Describe what code does with plain verbs: a function prints, returns, writes, or reads. Do not give code human verbs of thought or speech.
- Address the reader as "you" only in instructions.

## Security

Report vulnerabilities privately as [SECURITY.md](SECURITY.md) describes, not in an issue or pull request.

## License

iris is licensed under either the [MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE). A contribution is licensed under the same terms.
