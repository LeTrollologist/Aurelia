# Reproducible builds

Aurelia releases are currently built by the release orchestrator on the maintainer's Windows
machine. This page records the reproducibility assessment for issue #64.

## What is reproducible

- The source tree is pinned by commit and `Cargo.lock`; `cargo build --release -p aurelia-app
  --locked` is the intended source build.
- The workspace declares `rust-version = "1.87"` and `rust-toolchain.toml` uses the stable
  channel with clippy and rustfmt.
- Release assets are signed and every published asset has a SHA-256 digest in `SHA256SUMS.txt`;
  GitHub independently stores the same digest.

## What is not bit-reproducible today

- MSVC embeds timestamps and PDB paths in the PE/PDB; the portable executable is additionally
  processed by packaging and signing. A rebuilt binary is therefore not expected to be
  byte-identical to the published one.
- The release is not built by an independent CI runner. There is no third-party build
  attestation; the source, tag, signatures and published digests are the available evidence.
- `rust-toolchain.toml` follows the stable channel rather than pinning an exact patch release.

## Feasibility

A functional source rebuild is feasible on the documented Windows toolchain. A bit-for-bit
reproducible release would require:

1. pinning an exact Rust and MSVC/SDK toolchain;
2. deterministic linker settings and timestamp stripping;
3. a reproducible packaging step (zip timestamps, Inno Setup metadata);
4. an independent builder or CI attestation to compare against the published bytes.

Until those exist, treat the signed SHA-256 manifest as the integrity boundary, not a
reproducibility claim.
