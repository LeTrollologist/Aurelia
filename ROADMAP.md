# Aurelia · Roadmap

**Status:** v3.13.0 is the production release, shipped 2026-11-10 · Windows 10/11 x86-64 ·
Win32 + GDI, unsigned installer by permanent policy. v3.12.0 is the previous release.

**Baseline:** 378 tests passing, 2 ignored (soak and the sparse gate); formatting and Clippy
correctness are enforced by `scripts/quality.py`. Sparse 16-bit composite **0.73** against the
**0.60** budget on the last quality run. The GDI fallback remains an **8-bit presentation** of
16-bit documents; the native 10-bit canvas path is in development for v3.14.0.

**How to read this:** *Now* is the active maintenance/feature work, *Next* is queued in order,
*Later* is designed but unscoped, and *Not now* is a closed decision. Every feature ships with
tests and a written gate; P3 work starts with a short proposal in `docs/` before code.

---

## Maintenance — v3.13.x

**Status:** v3.13.0 is shipped; the audit milestone has no open issues. There is no scheduled
feature work in this line: patch releases are reserved for regressions, security fixes and
release-process defects.

**Exit gate:** a patch release requires a failing regression test first, the full
`python scripts/quality.py` gate, and the same signed/verifiable asset process as v3.13.0.

**Carry-over work from the 3.13 release:**

- Keep the formatting, Clippy-correctness, PE-metadata and ledger checks in the release gate.
- Keep local secrets in `.env`; never put a token or VirusTotal key on a command line.
- Exercise the manual hardware matrix before any patch: pen pressure/tilt, 100–200% DPI,
  two monitors, install/uninstall, and a clean-machine portable launch.

---

## Now — v3.14.0: High-bit-depth presentation (in progress)

**Status:** phase 2 is implemented in the development tree. The canvas and studio backdrop render
from native RGBA16 into packed 10-bit pixels and present through a D3D11/DXGI flip-model
swapchain when the output reports at least 10 bits per colour. The GDI path remains the fallback,
and selection/tracing/onion/soft-proof frames currently use it. The visible-banding gate still
needs a 10-bit display chain.

- D3D11 FL11.1 device, 10-bit composition and HWND swapchain present with an 8-bit fallback.
- Integrate the canvas present path and decide the egui overlay path.
- Gates: visible banding removed on 10-bit hardware, byte-identical 8-bit fallback, no startup or
  performance regression, mixed-monitor and Remote Desktop safety.
- Proposal and measurements: [docs/high-bit-depth-viewport.md](docs/high-bit-depth-viewport.md).

Colour management and 10-bit presentation are paired: neither alone is a professional-colour
claim.

---

## Next — queued in order

1. **Persistent brush and gradient preset library.** The import path now has an aggregate
   decoded-memory budget and session-only stamps. The next step is a versioned, bounded preset
   file with explicit import/export limits and migration tests.
2. **PSD import feasibility spike.** Read-only first: layer structure, masks, groups, text and
   adjustment layers. A separate compatibility matrix and fixture corpus are prerequisites;
   writing PSD is out of scope for the spike.
3. **Multi-document tabs.** A `DocumentSession` refactor; valuable, but it waits behind viewport
   and release reliability.
4. **Layer style extensions.** Inner shadow/glow, bevel and presets; only after a measured style
   pass, so effects are not added to a slow renderer.

---

## Later — larger bets, need their own proposal

| Candidate | Why it waits |
| :--- | :--- |
| **Animated WebP / MP4 export** | New encoders and timeline semantics; independent of the renderer. |
| **GPU acceleration** | Only when a measured CPU limit blocks a real workflow; the software path is the baseline. |
| **Cross-platform port** | `eframe`/`winit` frontend swap around the platform-agnostic crates; not small while the viewport is GDI-hosted. |
| **CMYK / print production** | Colour-managed print output, PDF/X and proofing; needs a profile and print-gamut design. |
| **32-bit float / linear-light editing** | Changes how values combine and invalidates the byte-identical 8-bit oracle; needs its own proposal and gate. |

---

## Explicitly not now — closed decisions

- **Forking or replacing egui, or a renderer rewrite.** Not justified by the current bug class.
- **Authenticode.** No public certificate path; Ed25519 artifact signatures plus SHA-256 manifests
  are the integrity boundary.
- **A cloud account, telemetry or auto-update service.** The local-first model is a product
  constraint, not a temporary implementation detail.
- **A second GPU renderer behind a hidden fallback.** Any acceleration must be measured,
  selectable and honest about its fallback.

---

## Continuous guardrails

1. **Performance:** sparse 16-bit ratio ≥ 0.60 (record paired runs); Luminosity rollback reference
   68.3 MPix/s full pass / 92.2 isolated. No optimization without a paired win.
2. **Quality:** `python scripts/quality.py` runs formatting, Clippy correctness, the release-tool
   pytest suite, the soak and the sparse gate. Everything runs locally; there is no GitHub
   Actions workflow and no cloud build. Releases remain local.
3. **Correctness:** the compositor oracle and the 8/16-bit digest suites gate every compositor,
   style or blend change. Project-file, clipboard, brush and IPC parsers keep explicit aggregate
   budgets and malformed-input tests.
4. **Release:** notes before tag, preflight rejects missing notes/BOM/mojibake, assets are signed
   and verified after upload, and the public ledger is regenerated. VirusTotal is advisory; a key
   belongs only in a gitignored `.env`.
5. **Manual hardware:** pen pressure/tilt, 100–200% DPI, two monitors, install/uninstall before a
   release candidate.

---

## Shipped

| Area | State |
| :--- | :--- |
| Production hardening (v3.13.0) | Invalid/cyclic hierarchies rejected; aggregate project/brush decode budgets; durable unique-temp saves; clipboard, DIB and Discord IPC bounds; owned `GWLP_USERDATA` window state; PE version metadata; release ledger |
| Colour management (v3.13.0) | ICC attach/assign/convert with one undo step, AUR `AICC`, PNG/JPEG/WebP profile embed+input, cached approximate soft proof; production |
| Clipboard, output & brushes (v3.12.0) | Paletted DIB and PNG clipboard, RGBA16 PNG export, styled clipping/Scale Styles/style-safe Merge Down, `.abr`/`.gbr` stamp import, presence-log privacy fix, full in-house quality gate |
| 16-bit integration (v3.11.0) | Copy Merged composites at 16 bits for 16-bit/mixed documents; sub-lattice round-trip tests for copy/paste and merged copy |
| Persistence & recovery (v3.10.0) | Path-hashed autosave names, whole-directory recovery scan, payload-bound section counts, truncation/bit-flip corpus, atomic-save failure test |
| Style performance (v3.9.0) | Content-bounds effect rendering: 3 ms small-art style pass at 1080p (was 129 ms); bit-identical to the full-canvas reference |
| Style reliability (v3.8.1) | Style edits mark the document dirty and commit on keyboard/wheel/leave/save; pinned blur boundary policy; metadata-only styled copy; DIB mask/alpha fixes |
| Layer styles | Stroke, drop shadow, outer glow at composite time; `ASTY` persistence; gesture-level undo |
| 16-bit sRGB documents | Storage, painting, compositing, filters, text, selection, transforms, merge, save/load |
| Editing workflows | Mask painting, selection combine modes, Preferences, History panel, tool/brush reset, persistent ruler/guide snapping |
| Viewport & stability | DPI-preserving framing, DPI-sized Navigator, premultiplied bilinear edges, perf/soak/eviction records |
| Platform & release | Windows x86-64, per-monitor V2 DPI, journaled local release tool, signatures, SHA-256 ledger, PE metadata |

---

## Working rules

1. **Correctness before throughput.** No optimization bundled with a correctness change.
2. **A test or it did not happen.** Viewport, release and 16-bit claims need deterministic tests or
   recorded measurements.
3. **One source of truth.** Version, notes, roadmap and issue state move together; the release
   tool rewrites the current-version lines it knows about.
4. **No silent fallback.** Unsupported operations refuse with a message; nothing downconverts
   behind the user's back.
5. **Unsigned is disclosed, not hidden.** Every release states the unsigned status and the
   verification path.

## References

- [docs/PERFORMANCE.md](docs/PERFORMANCE.md) — benchmarks, budgets, rollback numbers
- [docs/high-bit-depth-viewport.md](docs/high-bit-depth-viewport.md) — 10-bit proposal and spike
- [docs/COLOUR_MANAGEMENT.md](docs/COLOUR_MANAGEMENT.md) — colour pipeline and gates
- [docs/QUALITY.md](docs/QUALITY.md) — formatting, Clippy and test policy
- [docs/REPRODUCIBLE_BUILDS.md](docs/REPRODUCIBLE_BUILDS.md) — reproducibility assessment
- [RELEASE_PROCESS.md](RELEASE_PROCESS.md) — release mechanics and local trust model
