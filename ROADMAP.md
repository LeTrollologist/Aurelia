# Aurelia · Roadmap

**Status:** v3.12.0 shipped 2026-11-08 · Windows 10/11 x86-64 · Win32 + GDI, unsigned installer by
permanent policy.

**Baseline:** 338 tests passing, 2 ignored (soak and the 60-second sparse gate); clippy clean.
Sparse 16-bit composite **0.68** against the **0.60** budget on the last quality run. The viewport
is an **8-bit presentation** of 16-bit documents — a current limitation, not a closed decision:
high-bit-depth presentation is a tracked architecture track
([docs/high-bit-depth-viewport.md](docs/high-bit-depth-viewport.md)).

**How to read this:** *Now* is the next release, *Next* is queued in order, *Later* is designed but
unscoped, *Not now* is closed. Every feature ships with tests and a written gate; P3 work starts
with a short proposal in `docs/` before code.

---

## Now — v3.13.0: Colour management

**Status:** engine, document state and PNG embedding implemented in `main`. `IccProfile` parses
and validates profiles (8/16-bit RGBA `moxcms` transforms, sRGB identity no-op); `Document` can
attach/clear/convert; AUR v3 has an optional `AICC` section and PNG export embeds `iCCP`.
Remaining: JPEG/WebP embedding, UI assign/convert, soft proofing.

**Goal:** make colour meaning explicit. Untagged documents keep the current sRGB behaviour exactly;
tagged documents can carry an ICC profile, be converted into a chosen working space, and be soft
proofed in the viewport. Full proposal and gates:
[docs/COLOUR_MANAGEMENT.md](docs/COLOUR_MANAGEMENT.md).

| | |
| :--- | :--- |
| **Feature** | ICC profile attach/reference on documents and export; explicit **assign vs convert**; working-space selection; soft proofing approximation on the current 8-bit viewport. |
| **Hardening** | Byte-identical output for untagged 8-bit documents (the existing oracle is the gate); tagged files round-trip their profile; conversion is deterministic and documented at the 8/16-bit boundaries. |
| **Gates** | Untagged oracle unchanged; profile round-trip; a converted tagged document differs only by the documented transform; no silent profile dropping on export. |

**Out of scope:** CMYK/PDF-X print production, full HDR, and a complete colour-engine rewrite.

**First decision:** `lcms2` versus a deliberately small matrix/TRC parser with a documented profile
subset. The choice and the untagged-oracle gate belong in the proposal before code.

---

## Next — queued in order

1. **High-bit-depth presentation (10-bit DXGI).** Feasibility spike complete: D3D11 FL11.1 device
   and 10-bit composition/HWND swapchains create and present, with the 8-bit fallback working; the
   development display reports 8 bits per colour, so the visible-banding gate needs a 10-bit
   machine. Next is the canvas present path and egui overlay decision. Gates: visible banding
   removed on 10-bit hardware, byte-identical fallback, no startup/perf regression, mixed-monitor
   and Remote Desktop safety. Proposal:
   [docs/high-bit-depth-viewport.md](docs/high-bit-depth-viewport.md). Colour management is the
   paired milestone; neither alone is a professional-colour claim.

---

## Later — larger bets, need their own proposal

| Candidate | Why it waits |
| :--- | :--- |
| **Multi-document tabs** | `DocumentSession` refactor; valuable but not on the reliability/performance critical path. |
| **Colour management** | ICC profiles, working-space conversion, soft proofing; needs its own oracle design. |
| **Layer style extensions** | Inner shadow/glow, bevel, presets — explicitly after performance work; do not add effects to a slow renderer. |
| **PSD import/export** | Third-party file format and codecs; separate compatibility matrix and fixtures. |
| **Animated WebP / MP4 export** | New encoders and timeline semantics; independent of the renderer. |
| **GPU acceleration** | Only when a measured CPU limit blocks a real workflow; the software path is the baseline. |
| **Cross-platform port** | `eframe`/`winit` frontend swap around the platform-agnostic crates; not small while the viewport is GDI-hosted. |

---

## Explicitly not now — closed decisions

- **32-bit float / linear-light editing.** Changes how values combine and invalidates the
  byte-identical 8-bit oracle; needs its own proposal and gate.
- **Forking or replacing egui, or a renderer rewrite.** Not justified by the current bug class.
- **Authenticode.** No public certificate path; Ed25519 artifact signatures plus SHA-256 manifests
  are the integrity boundary.

---

## Continuous guardrails

1. **Performance:** sparse 16-bit ratio ≥ 0.60 (record paired runs); Luminosity rollback reference
   68.3 MPix/s full pass / 92.2 isolated. No optimization without a paired win.
2. **Stability:** `python scripts/quality.py` runs the soak and sparse gate on every release;
   eviction-integrity tests stay green.
3. **Correctness:** the 702-case compositor oracle and the 8/16-bit digest suites are gates for
   every compositor, style or blend change; the quality script also runs the release tool's
   pytest suite.
4. **Release:** notes before tag, preflight rejects missing notes/BOM/mojibake, assets signed and
   VirusTotal-scanned, published release re-downloaded and verified.
5. **Manual hardware:** pen pressure/tilt, 100–200% DPI, two monitors, install/uninstall before a
   release candidate.

---

## Shipped

| Area | State |
| :--- | :--- |
| 16-bit sRGB documents | Storage, painting, compositing, filters, text, selection, transforms, merge, save/load |
| Editing workflows | Mask painting, selection combine modes, Preferences, History panel, tool/brush reset, persistent ruler/guide snapping |
| Clipboard & smart layers | Cut/Copy/Copy Merged/Paste (lossless RGBA16 internal, DIBV5 interop), Adaptive auto-contrast blend mode |
| Viewport & stability | DPI-preserving framing, DPI-sized Navigator, premultiplied bilinear edges, perf/soak/eviction records |
| Clipboard, output & brushes (v3.12.0) | Paletted DIB and PNG clipboard, RGBA16 PNG export, styled clipping/Scale Styles/style-safe Merge Down, `.abr`/`.gbr` stamp import, presence-log privacy fix, full in-house quality gate |
| 16-bit integration (v3.11.0) | Copy Merged composites at 16 bits for 16-bit/mixed documents; sub-lattice round-trip tests for copy/paste and merged copy |
| Persistence & recovery (v3.10.0) | Path-hashed autosave names, whole-directory recovery scan, payload-bound section counts, truncation/bit-flip corpus, atomic-save failure test |
| Style performance (v3.9.0) | Content-bounds effect rendering: 3 ms small-art style pass at 1080p (was 129 ms); bit-identical to the full-canvas reference |
| Style reliability (v3.8.1) | Style edits mark the document dirty and commit on keyboard/wheel/leave/save; pinned blur boundary policy; metadata-only styled copy; DIB mask/alpha fixes |
| Layer styles | Stroke, drop shadow, outer glow at composite time; `ASTY` persistence; gesture-level undo |
| Platform & release | Windows x86-64, per-monitor V2 DPI, journaled release tool, signatures, VirusTotal scans |

---

## Working rules

1. **Correctness before throughput.** No optimization bundled with a correctness change.
2. **A test or it did not happen.** Viewport, release and 16-bit claims need deterministic tests or
   recorded measurements.
3. **One source of truth.** Version, notes, roadmap and issue state move together.
4. **No silent fallback.** Unsupported operations refuse with a message; nothing downconverts
   behind the user's back.
5. **Unsigned is disclosed, not hidden.** Every release states the unsigned status and the
   verification path.

## References

- [docs/PERFORMANCE.md](docs/PERFORMANCE.md) — benchmarks, budgets, rollback numbers
- [docs/16bit-plan.md](docs/16bit-plan.md) — the 16-bit record
- [docs/v3.8.0-LAYER_STYLES.md](docs/v3.8.0-LAYER_STYLES.md) — styles design and gates
- [RELEASE_PROCESS.md](RELEASE_PROCESS.md) — release mechanics and the production gate
