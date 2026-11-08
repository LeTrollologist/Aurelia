# Performance

Aurelia is CPU-rendered; there is no GPU fallback. Numbers are machine-dependent and should be
reproduced locally before they are quoted as guarantees.

## Recorded baseline

From the development machine used for the 3.0.x/3.1.0 work:

| Workload | Throughput |
| :--- | :--- |
| 4-layer 1080p composite | ~113 MPix/s |
| Same, with layer masks | ~92 MPix/s |

These are composite throughput figures, not end-to-end frame rates. UI frame timing also
includes egui layout, panel painting and the GDI blit.

## v3.7.0 baseline (#48, #49, #50)

Machine: Intel64 Family 6 Model 154 Stepping 3, 12 logical cores, rustc 1.93.1, release profile.
Measured 2026-11-07 at `9695548` plus the v3.7.0 working tree. Reproduce with the commands at the
bottom of this file.

### Sparse 16-bit ratio (#48)

`cargo test -p aurelia-core gate_2 --release -- --ignored --nocapture`, five paired rounds:

| Workload | u8 | Srgb16 | Ratio | Spread |
| :--- | ---: | ---: | ---: | ---: |
| sparse 10% | 126.6 MPix/s | 92.4 MPix/s | **0.75** | 0.64 .. 1.08 |

Budget is 0.60, so v3.7.0 has 25% headroom over the floor. The 0.75 exploratory target is met on
this run; it is recorded, not enforced, because the spread crosses 1.00 and a single fast round
must not become a gate.

### Composite path (#49 Luminosity)

Full run, median of five interleaved rounds (all 13 blend modes are timed in the same pass):

| Case | Throughput |
| :--- | ---: |
| plain stack | 106.4 MPix/s |
| with layer mask | 88.0 MPix/s |
| general, opaque | 88.8 MPix/s |
| general, clipping | 54.6 MPix/s |
| Normal (general) | 76.7 MPix/s |
| **Luminosity (general)** | **68.3 MPix/s** |

Isolated run (`bench_composite --filter luminosity`: one case, no interleaving with twelve other
modes): **92.2 MPix/s**. The gap between 68.3 and 92.2 is case-ordering/thermal effect, which is
why paired measurement uses the filter.

Luminosity is ~11% slower than Normal in the same pass. Profiling by inspection: the mode is the
only one that runs the full W3C SetLum + ClipColor sequence — two luminance dot products, a
channel shift, then min/max gamut clipping with two conditional rescales — rather than a
per-channel formula. The extra work is inherent to the definition, not an implementation
accident. No optimization was accepted for v3.7.0: nothing identified was both measurable and
correctness-neutral. The recorded rollback reference is **68.3 MPix/s in the full pass / 92.2
MPix/s isolated**; any future Luminosity change must beat that in paired runs and leave the
compositor oracle unchanged.

### Long-session soak (#50)

`cargo test -p aurelia-io --test soak --release -- --ignored --nocapture`, 120 cycles on a
192×192 RGBA16 document with a mask, 8 MiB history budget:

```
soak_record {"doc":"192x192 Srgb16 + mask","cycles":120,"undo_redo":718,
"history_steps":36,"history_bytes":8356828,"history_budget":8388608,
"live_start":9615883,"live_end":9646289,"peak":9669189,"growth":30406,
"growth_budget":4501122}
```

Interpretation: after a 20-cycle warm-up the live heap grew **30 KB** over the remaining 100
cycles (0.3% of the 4.5 MB allowance), while history sat at 99.6% of its 8 MiB budget — the
expected plateau for bounded retention, not a leak. Each cycle paints, applies a Gaussian blur,
renders a transform patch, performs 3 undos + 3 redos, and every 15th cycle saves and reloads the
project and compares the 16-bit bytes. Animation frames are added, sampled and deleted every 10th
cycle.

### Navigator after the DPI change (#45)

The thumbnail is no longer a fixed 320 px texture: it is sized from the dock's physical width
(0.92 × dock, clamped to 160–720 px), so a 200% monitor rebuilds at roughly twice the resolution
instead of upscaling. The per-refresh cost measured earlier (1.62 ms at 1080p, 2.72 ms at 4K for
320 px) scales with the square of the width; at the default 324 px dock the target is ~298 px, so
the default path is unchanged. The ~30 Hz live cap still applies during a stroke.

## Styled-layer cost (consistent workload, v3.9.0 planning baseline)

`cargo run --release -p aurelia-render --example bench_styles`, same machine, 3 warm-up runs and
20 measured runs per configuration. **Identical** art pattern and effect parameters at every layer
count: stroke 3 px, shadow offset (6, 6) with 8 px blur, glow 6 px. Raw samples:
[docs/benchmarks/style-baseline.json](benchmarks/style-baseline.json).

| Document | Layers | Style render | Composite (median) | Composite min / p95 | Viewport (median) |
| :--- | --: | --: | --: | --: | --: |
| 1920×1080 | 1 | 129 ms | 133 ms | 123 / 143 ms | 143 ms |
| 1920×1080 | 2 | 129 ms | 267 ms | 253 / 274 ms | 280 ms |
| 1920×1080 | 4 | 130 ms | 541 ms | 510 / 572 ms | 550 ms |
| 1920×1080 | 8 | 125 ms | 1030 ms | 1008 / 1081 ms | 1044 ms |
| 3840×2160 | 1 | 551 ms | 588 ms | 530 / 776 ms | 630 ms |
| 3840×2160 | 2 | 547 ms | 1095 ms | 997 / 1249 ms | 1163 ms |
| 3840×2160 | 4 | 526 ms | 2190 ms | 2134 / 2373 ms | 2219 ms |
| 3840×2160 | 8 | 788 ms | 6415 ms | 4190 / 7167 ms | 6492 ms |

Before the v3.8.1 cache-friendly vertical blur and reciprocal changes, the earlier harness
measured ~187 ms / ~769 ms / ~1109 ms / ~4676 ms for composite at 1080p×1, 1080p×4, 4K×1 and
4K×4. Those numbers used a per-layer-varying art pattern, so they are not directly comparable to
this table; the consistent-workload baseline above supersedes them for planning. (The published
v3.8.1 release notes cite the earlier harness.)

Removing the styled-layer buffer clone was inside the noise; style rendering is the cost.
Rendering is O(canvas × effects) and stays a budget item until the v3.9.0 content-bounds work
lands. Until then, treat several styled layers at 4K as not viewport-interactive; plain and
mask-only documents are unaffected.

## 16-bit completion baseline

Measured at `checkpoint/v3.4.0-16bit-complete` (`bfa6709`) on Intel64 Family 6 Model 154
Stepping 3, rustc 1.93.1, release profile, five paired rounds. This is the stabilization
baseline, not a new target:

| Workload | Throughput |
| :--- | ---: |
| General-path composite, 4-layer 1080p, opaque | **105.4 MPix/s** |
| Slowest blend mode (Luminosity) | 82.5 MPix/s |
| Plain stack (8-bit fast path) | 163.7 MPix/s |
| Masked stack | 133.2 MPix/s |
| Brush, `Srgb8` | 1208 operations/s |
| Brush, `Srgb16` | 1263 operations/s (+5%) |
| Gate 2 sparse ratio (`Srgb16` / u8) | **0.67** (u8 175.8, `Srgb16` 113.8) |

The older ~113 / ~92 MPix/s figures are the fast-path composite baseline; the 105.4 general-path
number is the like-for-like path a 16-bit document uses. The gate-2 budget for sparse is 60% of
u8, so 67% passes with limited headroom — sparse is the first performance item in the
stabilization phase.

## Navigator thumbnail budget

The viewport keeps the Navigator live while painting. A 320 px thumbnail rebuilt from the latest
composite costs, on the same machine, median of 7 rounds x 200 iterations:

| Document | Median per refresh | Maximum refresh rate |
| :--- | ---: | ---: |
| 1920x1080 | **1.62 ms** | ~616/s |
| 3840x2160 | **2.72 ms** | ~368/s |

At 4K this is ~16% of a 16.7 ms frame, so the app caps live rebuilds at **~30 Hz while a stroke is
in progress**; the final refresh after mouse-up is immediate. Idle frames still do no work.

## Reproduce

```powershell
cargo run --release -p aurelia-render --example bench_composite
# One case only, for paired A/B measurement:
cargo run --release -p aurelia-render --example bench_composite -- --filter luminosity
cargo run --release -p aurelia-render --example bench_composite -- srgb16 --filter luminosity
cargo run --release -p aurelia-render --example bench_brush
cargo run --release -p aurelia-render --example bench_navigator
cargo test -p aurelia-core gate_2 --release -- --ignored --nocapture
cargo test -p aurelia-io --test soak --release -- --ignored --nocapture
```

Run on the same machine and power profile; laptop power saving, background load and debug builds
change results substantially.

## What is not claimed

- “60+ FPS” is a target for typical editing, not a guarantee for every canvas/layer combination.
- “Zero allocation” applies to selected composite/brush passes, not the whole application.
- Large documents are memory-bound; see [MEMORY_AND_UNDO.md](MEMORY_AND_UNDO.md).
