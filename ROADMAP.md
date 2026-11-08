# Aurelia · Roadmap

**Status:** v3.8.1 shipped 2026-11-08 · Windows 10/11 x86-64 · Win32 + GDI, unsigned installer by
permanent policy.

**Baseline:** 301 tests passing, 2 ignored (soak and the 60-second sparse gate); clippy clean.
Sparse 16-bit composite **0.75** against the **0.60** budget. The viewport is an **8-bit
presentation** of 16-bit documents — a recorded decision, not an open item
([docs/RGBA16_PRESENTATION.md](docs/RGBA16_PRESENTATION.md)).

**How to read this:** *Now* is the next release, *Next* is queued in order, *Later* is designed but
unscoped, *Not now* is closed. Every feature ships with tests and a written gate; P3 work starts
with a short proposal in `docs/` before code.

---

## Now — v3.9.0: Styled-layer performance

**Goal:** make layer styles interactive at the sizes Aurelia claims to handle. Styled layers are
currently O(canvas × effects) and cost ~130 ms per layer at 1080p / ~550 ms at 4K (consistent-workload bench).

| | |
| :--- | :--- |
| **Feature** | Content-bounds evaluation: render each style only inside the layer's nontransparent bounds, expanded by the effect's true support (blur passes, shadow offset, stroke width), then composite that sub-rect. |
| **Hardening** | Pixel-equivalent output against the full-canvas reference (including transparent-edge behaviour at canvas borders and layers whose art touches an edge); mask caching keyed on layer/style revisions; no full-canvas allocations for small objects. |
| **Gates** | Bit-identical composite vs the reference on randomized fixtures; v3.8.1-style bench improves materially with identical effect parameters and workload across layer counts; existing 301 tests unchanged. |

**Out of scope:** new style effects, GPU work, changing the blur boundary policy. The proposal is
[docs/v3.9.0-STYLE_PERFORMANCE.md](docs/v3.9.0-STYLE_PERFORMANCE.md).

---

## Next — queued in order

1. **AUR persistence and recovery hardening.** Fuzz the optional sections (`AED1`, `ASTY`), crash
   recovery with multiple sections, save/load idempotency, and a permanent historical fixture
   corpus. Gate: no panic or misparse on a crafted corpus; recovery restores the same bytes.
2. **Viewport correctness and invalidation.** Audit style/transform/mask invalidation, pan/zoom
   edge cases and DPI changes after v3.8/v3.9; add a deterministic "what was redrawn" test seam.
3. **Clipboard and external interoperability.** Complete DIB coverage (paletted inputs), add a
   PNG clipboard format where supported, and test against real producer fixtures.
4. **Deferred style semantics, when their workflows matter:** clipping to a styled base, a "Scale
   Styles" option, and Merge Down compositing styles before it is exposed.
5. **Interop batch — brush imports and native 16-bit PNG export.**
   - `.abr` / `.gbr` stamp decoding through the existing `aurelia-core::assets` seam
     (source → decode boundary → RGBA stamp), with malformed-input tests.
   - Native RGBA16 PNG export using `composite_layers_srgb16`; PNG is the only format that can
     carry the precision, so JPEG/WebP stay 8-bit with a documented reason.

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
- **Native RGBA16 viewport presentation.** Investigated and rejected; GDI/egui are 8-bit paths and
  a real 16-bit display needs D3D/DXGI. Re-open only with a 10-bit-capable target and a measured
  visible-fidelity test.
- **Authenticode.** No public certificate path; Ed25519 artifact signatures plus SHA-256 manifests
  are the integrity boundary.

---

## Continuous guardrails

1. **Performance:** sparse 16-bit ratio ≥ 0.60 (record paired runs); Luminosity rollback reference
   68.3 MPix/s full pass / 92.2 isolated. No optimization without a paired win.
2. **Stability:** run `cargo test -p aurelia-io --test soak --release -- --ignored` before each
   stability release; eviction-integrity tests stay green.
3. **Correctness:** the 702-case compositor oracle and the 8/16-bit digest suites are gates for
   every compositor, style or blend change.
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
