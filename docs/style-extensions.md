# Layer style extensions: inner shadow, inner glow, bevel

**Status:** proposal + implementation record for the v3.18 feature line.
**Scope:** additive effects in the existing composite-time style pipeline. No renderer
replacement, no document-model change, no new dependency.

## Baseline (measured before the feature)

`cargo run --release -p aurelia-render --example bench_styles` on the development machine,
warm-up 3, 20 measured runs (medians; min/p95 in parentheses):

| Fixture | Style pass | Composite | Viewport |
| :--- | ---: | ---: | ---: |
| 1920×1080, 1 layer, small art | **4.13 ms** (3.08 / 5.50) | 9.92 ms | 24.19 ms |
| 1920×1080, 1 layer, large art | 98.35 ms (70.48 / 125.80) | 90.08 ms | 114.11 ms |
| 1920×1080, 8 layers, large art | 120.45 ms | 1019.41 ms | 1031.25 ms |
| 3840×2160, 1 layer, small art | 15.03 ms (9.96 / 17.03) | 36.51 ms | 93.29 ms |
| 3840×2160, 1 layer, large art | 514.92 ms | 556.21 ms | 662.52 ms |

The historical record in `docs/PERFORMANCE.md` (79 ms large / 3 ms small at 1080p) was taken on
a different run; the numbers above are the baseline for this work on this machine. The style pass
still dominates the *small-art* case only mildly (4 ms of a 24 ms viewport); the large-art case is
bounded by full-canvas effect support, not by the effect count.

## What ships in this increment

1. `InnerShadowStyle`, `InnerGlowStyle`, `BevelStyle` in `aurelia-core::layer_style`, with
   `LayerStyles` growing three optional fields (serde-defaulted, so in-memory and JSON callers are
   unaffected).
2. Rendering in `render_rect`, after the layer is painted, in the fixed order inner shadow →
   inner glow → bevel. Inner effects **tint** existing pixels and never change alpha; partial
   coverage stays fractional (`coverage = blurred_inverse * alpha`, then a colour blend).
3. Bevel: deterministic normal field from the gradient of a blurred silhouette (central
   differences, clamped borders), Lambert term against a light direction in degrees (225° =
   upper-left). Holes and islands produce their own local normals; one-pixel details are finite.
4. `styled_rect` support: inner effects expand the bounded rectangle by the same margin as outer
   effects, because they blur the *inverse* silhouette (which is 1 over the empty area). Without
   that margin the bounded renderer disagreed with the full renderer by one unit.
5. ASTY **version 2** (below).
6. UI controls in the Layer Styles panel, wired through the existing
   begin/finish gesture mechanism, so one drag is one undo step.

## ASTY v2

| | v1 (shipped ≤ 3.17.5) | v2 (this line) |
| :--- | :--- | :--- |
| Version byte | 1 | 2 |
| Flags | stroke, drop shadow, outer glow | + inner shadow (8), inner glow (16), bevel (32) |
| Reader | v1 only | v1 **and** v2; unknown versions ignored |
| Writer | v1 | v2 |

Compatibility rules:

- New builds read v1 and v2 and always write v2.
- The AUR v3 container is unchanged; ASTY stays an optional trailing section.
- An unknown ASTY version is ignored, so the document still loads without styles.
- **Loss on resave:** a build that predates v2 ignores the section and, if the user saves the
  project, the new effects are dropped. This is the same behaviour older builds already had for
  unknown trailing sections; release notes state it explicitly and users are not silently
  downgraded within a single build.

## Gates

1. Existing style digests unchanged when none of the new effects is enabled (opaque path
   byte-identical; `scale_canvas_baselines` and the existing bounded-equivalence suite).
2. Bounded renderer equals the full renderer for randomized fixtures including all six effects.
3. Alpha is preserved exactly by every inner effect; transparent pixels stay colourless.
4. Inner shadow: fractional alpha scales the effect, holes cast inward, far pixels untouched.
5. Bevel: upper-left lit and lower-right shaded for 225°; deterministic on one-pixel details;
   16-bit has the same directional behaviour (equivalent, not byte-identical, across storage).
6. ASTY v1 payload parses with the new fields `None`; v2 round-trips; unknown version ignored.
7. Performance: record the style pass before/after for the small-art fixture; no regression in
   the existing digest suite.

## Style preset library (`styles.aurp`)

A sibling library to `presets.aurp`, with its own schema so neither can invalidate the other:

```
"ASRP" | u16 version(1) | u32 count | entry...
entry: u32 name_len | UTF-8 name | u32 json_len | JSON(LayerStyles)
```

- Bounds: 256 presets, 200-byte names, 64 KiB JSON each, 2 MiB payload; every count and length is
  checked before allocation.
- Atomic save: unique sibling temp file, `sync_all`, rename; a failed save removes the temp file
  and leaves the previous library untouched.
- Version 1 is the only accepted version; unknown versions are rejected with an explicit error
  rather than reinterpreted.
- Storage path: `%LOCALAPPDATA%\Aurelia\styles.aurp` (an `AURELIA_STYLE_PRESETS_PATH` override
  exists for portable installs and tests).
- Panel: save the active stack under a name, apply any preset as one undoable style edit, delete,
  and import/export the library through the host file dialogs.

## Deferred

- Rename and reorder in the preset panel (delete + save under a new name covers the workflow).
- Style presets inside brush preset categories (the two libraries stay separate by design).

## Non-goals

- No new blend-mode or compositing semantics; styles still evaluate at composite time.
- No baked-in style rasterisation except the existing explicit "rasterise" paths.
- No change to 8-bit storage precision or to the 16-bit working precision.
