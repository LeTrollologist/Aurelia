# Colour management — proposal

Status: **proposal accepted, engine seam implemented**. This document records the library decision
and the staged plan; persistence, UI and soft proofing are future phases.

## Decision: `moxcms`, not `lcms2`, not a hand-rolled parser

`moxcms` 0.8.1 (BSD-3-Clause OR Apache-2.0) is the colour engine.

- **Pure Rust, no C toolchain.** `lcms2` needs a C compiler/library in the build and release
  machine; `aurelia-core` stays buildable with cargo alone.
- **Right API shape.** `ColorProfile::new_from_slice`, `create_transform_8bit/16bit` with an RGBA
  layout, and `TransformOptions`; exactly the two storage widths Aurelia has.
- **Available offline and auditable.** The crate is small, already in the local registry, and its
  licence is GPLv3-compatible.
- **Hand-rolled ICC is rejected.** Matrix/TRC profiles are easy; LUT profiles, chromatic adaptation,
  rendering intents and malformed-profile hardening are not. A hand-rolled subset would either
  refuse most real profiles or become a second colour engine to maintain.

The seam is `aurelia-core::color_management::IccProfile`: parsing/validation, retained bytes for
round-trip, sRGB identity, and 8/16-bit RGBA conversion. `moxcms` types do not leak into the
document model.

## Model

- `Document` gains an optional profile reference (raw ICC bytes + a parsed handle).
- **Untagged** documents keep today's sRGB behaviour and are **not converted**; the existing
  8-bit oracle and the 16-bit paths stay byte-identical.
- **Assign profile** changes the interpretation without touching pixels.
- **Convert to profile** runs an explicit transform; this is the only operation that moves pixel
  values.
- A working-space preference (sRGB initially) decides what "convert" targets.

## Persistence

- AUR v3: optional trailing `AICC` section after `ASTY`, length-prefixed/versioned, absent or
  damaged sections ignored. No container-version bump.
- PNG: read/write the `iCCP` chunk. JPEG: read/write APP2 ICC segments. WebP: `ICCP` chunk.
  “No silent profile dropping”: export either embeds the profile or reports that the format cannot.
- Existing files without a profile load unchanged.

## Display

- The current viewport is an 8-bit sRGB presentation. Soft proofing there is an **approximation**:
  convert the proof-space preview and mark it clearly.
- Real 10-bit presentation is the separate DXGI track; colour management does not pretend the
  display is wide-gamut before that lands.

## Gates

| Gate | Evidence |
| :--- | :--- |
| Untagged 8-bit output unchanged | Existing 702-case oracle and digest suites pass without re-baselining. |
| Untagged 16-bit unchanged | `v*257` lattice and 16-bit tests unchanged. |
| Profile round-trip | `AICC`/`iCCP`/APP2 bytes survive load → save → load exactly. |
| Explicit conversion only | Assign never moves pixels; convert is the only mutating path and is covered by tests. |
| Malformed profiles | Empty, truncated and garbage profiles are rejected with structured errors; no panic. |
| No silent drop | Export either embeds a profile or returns a documented “not supported by this format” status. |

## Phases

1. **Engine seam** — implemented: parse/validate ICC, sRGB identity, 8/16-bit RGBA conversion,
   malformed tests.
2. **Document state + AUR `AICC`** — attach/clear/assign/convert; persistence round-trip.
3. **PNG `iCCP`** — embed on export, read on import; JPEG/WebP follow.
4. **UI** — document profile status, assign/convert actions, working-space preference.
5. **Soft proofing** — approximate 8-bit preview with an explicit badge.
6. **10-bit presentation** — paired with the DXGI track, not a prerequisite for phases 1–5.

## Non-goals

- CMYK/PDF-X print production and spot colours.
- HDR/wide-gamut display claims before the presentation track lands.
- Linear-light editing; that is a separate architecture decision.
