# Quality policy

The release gate is `python scripts/quality.py`. Its checks are blocking; a release is not cut
from a tree that fails any of them.

## Formatting

`cargo fmt --all -- --check` must pass. Formatting is not advisory: a release commit that changes
Rust source must be rustfmt-clean.

## Clippy

`cargo clippy --workspace --all-targets -- -D clippy::correctness` is blocking. Correctness lints
(including `erasing_op`) are errors. Style lints remain advisory until the existing style backlog
is cleared; do not add new correctness suppressions. When a style lint becomes cheap to fix, fix
it rather than extending the allowlist.

## Tests

- `cargo test --release --workspace` must pass.
- `python -m pytest tools/release/tests -q` must pass.
- The ignored sparse-16-bit throughput/memory gate and the long-session soak run explicitly and
  must pass their assertions.

## Release notes

Release notes may only quote a test count or a passing check that the quality gate actually
reported for that tree. Do not claim `clippy clean`, a toolchain version, or a scan result that
was not produced by the current run.
