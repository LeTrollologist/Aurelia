# Aurelia v3.13.0 — engineering case study

Aurelia is a local-first Windows illustration and animation studio: six Rust crates, a CPU
software renderer, and a deliberately narrow Win32 FFI host. The v3.13.0 release is a useful
case study because most of its work is about making an already capable application defensible,
durable and distributable.

## Secure binary parsing and resource limits

The project format is an untrusted binary input, so decoding is bounded at several layers:

- AUR hierarchy decoding rejects cycles, self-parenting, duplicate IDs and invalid child links
  with `CorruptData` instead of recursing.
- Aggregate limits cap documents at 4096 layers, 512 Mi pixels and 2 GiB of decoded layer
  storage; the budget is charged before large allocations.
- Brush imports share a 63 Mi pixel / 252 MiB decoded budget across all stamps in one file.
- Clipboard DIB imports are capped at 64 megapixels, with checked width, height, stride and
  decoded-length arithmetic.
- The Discord IPC reader requires a valid opcode and caps frames at 1 MB.

Each boundary has malformed-input and over-budget regression tests, not just a happy-path test.

## Rust ownership at the Windows boundary

The Win32 host previously kept a `static mut` pointer to the application state. v3.13.0 passes an
owned `Box<AureliaDesktopApp>` through `CreateWindowExW`, stores it in `GWLP_USERDATA` during
`WM_NCCREATE`, and clears and drops it in `WM_DESTROY`. The message loop has an explicit fallback
cleanup. This keeps the unsafe surface at the FFI boundary while making the window-state lifetime
explicit and preventing dispatch from reading a stale global.

## Durable persistence

Project saves use a unique sibling temporary file, write through the normal serializer,
`sync_all()` before replacement, and rename atomically. Failure paths remove the temporary file
and leave the previous project intact. The parent directory is synced where the platform supports
it. The save path is exercised by failure-injection tests and a “previous file still opens” check.

## Cryptographically verified distribution

- Ed25519 detached signatures cover both binaries and the `SHA256SUMS.txt` manifest.
- The portable executable carries PE `FileVersion`/`ProductVersion` metadata generated from the
  Cargo package version.
- The release ledger records published asset hashes; release notes are generated from real gate
  results.
- VirusTotal results are published per artifact. The v3.13.0 installer has four advisory
  detections; the portable zip and unpacked executable are 0/75. The portable archive is offered
  as the smaller trust surface rather than dismissing the installer findings.
- Authenticode is not available; the documentation distinguishes artifact signatures from a
  Microsoft-trusted publisher identity.

## Quality gates and reproducibility

`python scripts/quality.py` runs formatting, Clippy correctness, the Rust workspace suite, the
release-tool pytest suite, the sparse-16-bit gate and the soak. The current tree has 377 Rust
tests passing with 2 ignored. Releases are built locally with a journal-backed orchestrator;
there is no GitHub Actions or cloud build. The reproducibility assessment records what is and is
not bit-for-bit reproducible, including MSVC timestamps and packaging.

## What this demonstrates

- Treating file formats as hostile input with explicit aggregate budgets.
- Using Rust ownership to remove global mutable state at an FFI boundary.
- Designing persistence for crash and failure cases, not only successful saves.
- Shipping verifiable artifacts with transparent, reproducible release records.
- Making quality gates part of the release process rather than a claim in a README.

## Links

- [Aurelia v3.13.0 release](https://github.com/LeTrollologist/Aurelia/releases/tag/v3.13.0)
- [Release notes](release-notes/v3.13.0.md)
- [Security model](docs/SECURITY_MODEL.md)
- [Quality policy](docs/QUALITY.md)
- [Reproducibility assessment](docs/REPRODUCIBLE_BUILDS.md)
