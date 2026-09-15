# Development

Code style follows the `rustfmt.toml` file.

## Commit messages

The `meli` project requires per-commit developer sign-offs just like the Linux
project.

Quoting <https://wiki.linuxfoundation.org/dco>:

> The DCO is a per-commit sign-off made by a contributor stating that they agree
> to the terms published at <https://developercertificate.org> for that
> particular contribution.
>
> When creating a commit with the Git CLI, a sign-off can be added with the -s
> option: <https://git-scm.com/docs/git-commit#git-commit--s>. The sign-off is
> stored as part of the commit message itself, as a line of the format:
>
> ```text
> Signed-off-by: Full Name <email>
> ```

The sign-off line must be the final line of your commit message, without any
empty lines after.

Always use `git commit -s` to have `git` add a proper sign-off trailer line in
your commit message.

## CI

All pull requests require CI checks to pass.
You can run the same checks locally without a CI runner.

The CI workflows are written to execute the following `Makefile`s:

- [`.gitea/Makefile.build`](.gitea/Makefile.build)
  Runs build checks with `cargo-check`, `cargo-test --no-run`, `make
  build-rustdoc` and also runs cargo tests with `cargo-nextest` and `rustdoc`
  tests with `make test-docs`.
- [`.gitea/Makefile.lint`](.gitea/Makefile.lint)
  Performs linter checks with `rustfmt`, `clippy`, `cargo-msrv` and `cargo-derivefmt`.
- [`.gitea/Makefile.manifest-lint`](.gitea/Makefile.manifest-lint)
  Performs linter checks for manifest files with `cargo-sort` and the
  [`check_debian_changelog.sh`](./scripts/check_debian_changelog.sh) script.

This means you don't have to run the CI with a pull request to see if the
checks pass, you can do the equivalent checks locally with something like:

```sh
for m in .gitea/Makefile.*; do
  make -f "${m}" || break
done
```

Or run all checks in a specific `Makefile`:

```sh
make -f .gitea/Makefile.lint
```

Or run a specific check in a specific `Makefile`:

```sh
make -f .gitea/Makefile.lint clippy
```

## Trace logs

Enable trace logs to `stderr` with:

```sh
export MELI_DEBUG_STDERR=yes
```

This means you will have to to redirect `stderr` to a file like `meli 2> trace.log`.

Tracing is opt-in by build features:

```sh
cargo build --features=debug-tracing,imap-trace,smtp-trace
```

## use `.git-blame-ignore-revs` file _optional_

Use this file to ignore formatting commits from `git-blame`.
It needs to be set up per project because `git-blame` will fail if it's missing.

```sh
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

## Formatting with `rustfmt`

```sh
make fmt
```

## Linting with `clippy`

```sh
make lint
```

## Terminal UI architecture

The UI sits on two libraries, glued by an edge adapter:

- `crossterm` owns terminal I/O: raw mode, alternate screen, mouse capture,
  bracketed paste, and the input event parser (`meli/src/terminal/input.rs`,
  `meli/src/terminal/screen.rs`).
- `ratatui` owns layout solving and border rendering. The top-level chrome
  (tab bar, status bar, pane splits, dialog placement) computes its areas
  through ratatui `Layout` helpers exposed by
  `meli/src/terminal/ratatui_bridge.rs`.
- meli's own `CellBuffer` remains the painting target. Components draw into
  it as before, the screen keeps flushing dirty segments itself
  (`draw_horizontal_segment` in `meli/src/terminal/screen.rs`), and
  `ratatui::Terminal`'s full-frame diff flush is deliberately not used.

`meli/src/terminal/ratatui_bridge.rs` is the seam between the two worlds:
color and attribute conversions, whole-buffer blits between `CellBuffer` and
ratatui's `Buffer`, `Area`/`Rect` converters, key/mouse translation from
crossterm events onto meli's `Key` vocabulary, and `encode_key`, which
re-encodes a `Key` into the raw bytes the pre-migration reader produced. The
embedded terminal and the `ThreadEvent::Input` contract depend on those
bytes.

## Testing

```sh
make test
```

How to run specific tests:

```sh
cargo test -p {melib, meli} (-- --nocapture) (--test test_name)
```

### Golden snapshot tests

`meli/src/golden.rs` pins the rendering of the major UI surfaces (listings,
pager, composer, dialogs, tab and status bar chrome) to golden files under
`meli/tests/golden`. Each test draws a component into a `Screen<Virtual>` and
compares the serialized cell buffer byte-for-byte against the committed
file.

Run them like this:

```sh
XDG_DATA_HOME=/tmp/meli-test-xdg cargo test -p meli golden
```

Point `XDG_DATA_HOME` at a writable scratch directory: mock contexts resolve
XDG paths from the process environment, and a scratch directory keeps the
run from touching your real user data. Two switches control recording:

- `MELI_UPDATE_GOLDEN=1` re-records goldens instead of asserting. Review the
  resulting diff before committing; an unexpected diff means rendering
  changed.
- `MELI_GOLDEN_DIR=<path>` redirects where goldens are read from and written
  to (default `meli/tests/golden`), so a re-record can target a scratch
  directory and be diffed against the committed corpus.

## Profiling

```sh
perf record -g target/debug/meli
perf script | stackcollapse-perf | rust-unmangle | flamegraph > perf.svg
```
<!--  -->
<!-- ## Running fuzz targets -->
<!--  -->
<!-- Note: `cargo-fuzz` requires the nightly toolchain. -->
<!--  -->
<!-- ```sh -->
<!-- cargo +nightly fuzz run envelope_parse -- -dict=fuzz/envelope_tokens.dict -->
<!-- ``` -->

## Coverage

```sh
export RUSTFLAGS="-Cinstrument-coverage" ; export LLVM_PROFILE_FILE="meli-%p-%m.profraw"
make -f .gitea/Makefile.build cargo-test
make -f .gitea/Makefile.build rustdoc-test
grcov . -s . --binary-path ./target/debug/ -t html --branch --ignore-not-existing -o ./target/debug/coverage/
```

And inspect `target/debug/coverage/index.html`.


Clean profile files with:

```sh
find . -name '*.profraw' -delete
```
