# High-bit-depth viewport presentation — architecture proposal

Status: **phase 2 implemented in the development tree (v3.14.0); visual and mixed-monitor gates
still pending.** This is not a released feature yet, and it is not a promise of immediate
delivery; it defines the target, the path, and the gates so the gap is tracked as architecture
work rather than closed as out of scope.

## The distinction

- **Editing precision:** storage, compositing, filters, transforms, selections and layer styles run
  at working precision and preserve RGBA16. That part is real and tested.
- **Presentation precision:** the viewport currently converts the result to 8 bits per channel
  before it reaches the screen. An artist cannot see 16-bit gradients or judge banding from inside
  Aurelia, even though the data is there.

Calling the engine "8-bit" because of the viewport would be wrong. Calling the display pipeline
finished would also be wrong.

## Current pipeline

```
CPU composite (f32 working precision, RGBA16 storage)
  → native RGBA16 composite
  → packed R10G10B10A2_UNORM canvas + 8-bit UI/guide overlay
  → D3D11/DXGI flip-model present (10-bit output only)
  → GDI SetDIBitsToDevice fallback (byte-compatible)
```

GDI blits have no 16-bit-per-channel format, and egui textures are RGBA8. The precision is lost in
the final present step, not in editing.

## Implemented in the development tree

- `SoftwareRenderer::render_canvas_srgb16` composites from the native RGBA16 path and writes
  packed `R10G10B10A2_UNORM` pixels. The 8-bit GDI path is unchanged.
- `TenBitPresenter` creates a D3D11 device, an HWND flip-model swapchain and a dynamic 10-bit
  texture, then uploads/copies/presents each frame. It is enabled only when the containing output
  reports at least 10 bits per colour; `AURELIA_FORCE_10BIT=1` is a hardware-diagnostics override.
- Selection tint, tracing reference, onion skins and soft proof are now blended in 10-bit space;
  the 16-bit proof path uses `simulate_proof_rgba16` and a separate cached proof buffer.
- The canvas and studio backdrop are 10-bit. UI, grid, rulers and guides remain 8-bit and are
  expanded into the 10-bit frame; canvas pixels are selected per-pixel only where the 8-bit
  overlay did not change them.
- DPI and display changes recreate the presenter against the new client size; the fallback still
  covers creation failure. Mixed-monitor hardware verification is phase 3 work.

For diagnostics only, `AURELIA_FORCE_10BIT=1` bypasses the output-depth check. It still falls back
if device/swapchain/texture creation fails, and it does not make an 8-bit panel show 10-bit
information.

## Target

**10-bit-per-channel SDR presentation on capable hardware**, with the existing path as the
fallback. True 16-bit scanout is not a realistic Windows target; 10-bit is the practical step that
removes visible banding on a 10-bit panel and matches what professional displays accept.

This is presentation only. Working precision, colour maths and the 8-bit oracle stay unchanged for
the fallback path.

## Recommended path: D3D11 + DXGI flip model

1. **D3D11 device and DXGI flip-model swapchain** with `DXGI_FORMAT_R10G10B10A2_UNORM` on a
   composition swapchain, sRGB colour space (`DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709`), no HDR
   metadata for the first milestone.
2. **Canvas present path**: convert the compositor's 16-bit output to 10-bit (rounded, no dither)
   in the upload buffer; keep the CPU compositor. This is a present-path change, not a renderer
   rewrite.
3. **UI overlay**: egui stays 8-bit. Either composite the UI in a separate surface over the
   10-bit canvas (DirectComposition) or draw egui into the 10-bit target in a second pass. The
   second is simpler but touches egui's renderer integration; the first keeps egui untouched.
4. **Fallback**: if device creation, format support or output bit depth fails, use the current GDI
   path unchanged. Remote Desktop and GPU-less VMs must take the fallback without a crash or a
   startup delay.

Alternative considered: a full `wgpu`/`eframe` port. It would solve presentation and
cross-platform at once, but it is a renderer/framework rewrite and should not be the first step
while the GDI path remains the compatibility baseline.

Direct2D alone does not give a 10-bit present path without a DXGI swapchain underneath, so it is
not simpler than the D3D11 route.

Dithering the 16→8 conversion was rejected for fidelity reasons: it trades banding for visible
noise at 100% zoom and prevents judging colour accurately.

## Feasibility spike (2026-11-08)

`tools/dxgi-spike` is a standalone, dependency-free diagnostic (raw COM, no `windows` crate) that
enumerates the output, creates a D3D11 device, and attempts composition and HWND swapchains at
`R10G10B10A2_UNORM` and `B8G8R8A8_UNORM`.

Run:

```
cargo run --offline --manifest-path tools/dxgi-spike/Cargo.toml
```

Result on the development machine:

```
d3d11: ok (driver=hardware, feature_level=0xb100)
dxgi: factory2 ok
display \\.\DISPLAY1: bits_per_color=8 color_space=0 max_luminance=270.0
composition swapchain 10-bit: created (present hr=0x00000000)
composition swapchain 8-bit fallback: created (present hr=0x00000000)
hwnd swapchain 10-bit: created (present hr=0x00000000)
hwnd swapchain 8-bit fallback: created (present hr=0x00000000)
result: device=ok 10-bit-swapchain=ok 8-bit-fallback=ok 10-bit-display=no
```

What this proves:

- A D3D11 hardware device (feature level 11.1) is available and a 10-bit composition **and** HWND
  swapchain can be created and `Present` returns S_OK, alongside the 8-bit fallback.
- The attachment is an 8-bit-per-colour output, so the **visible-banding gate cannot be evaluated
  on this machine**. No window was shown and no DirectComposition commit was performed, so this is
  format/device feasibility, not proof of visible 10-bit scanout.

What it does not prove: actual 10-bit scanout, colour-managed output, resize/DPI/device-loss
behaviour, or egui overlay composition. Those remain the integration phase and need a machine with
a 10-bit display chain.

## Dependency: colour management

10-bit output without profile-aware colour management is only half of a professional workflow.
Soft proofing and ICC conversion are a separate milestone (see ROADMAP). The presentation work
should land first because it is the visible gap, but neither alone earns an "end-to-end
high-precision studio" claim.

## Gates

| Gate | Status | Evidence |
| :--- | :--- | :--- |
| **Visible improvement** | **Pending** | On a 10-bit display chain, a vertical 16-bit gradient shows no 8-bit banding; a measured step test distinguishes 8-bit and 10-bit presentation. |
| **Fallback is unchanged** | **Pass (code path)** | With DXGI unavailable, output is byte-identical to the current GDI path and all viewport digest tests pass. |
| **No startup/perf regression** | **Pending** | Device creation is lazy or bounded; first-frame and resize timings stay within the recorded viewport budget. |
| **Mixed monitors** | **Partial** | DPI/display changes recreate the presenter; mixed 10-bit/8-bit hardware verification is still open. |
| **Remote/VM safety** | **Pending hardware** | Remote Desktop and WARP-only environments fall back without dialogs, crashes or repeated retries. |
| **Selection/tracing/onion/proof** | **Pass (unit)** | The 10-bit canvas blends all four overlay families; selection tint and the 16-bit proof path have tests. |
| **Colour correctness** | **Pass (unit)** | 10-bit values are converted from sRGB-encoded 16-bit data without a linear-light detour; an RGBA16 canvas test distinguishes 0 and 128 red at 10 bits. |

## Phases

1. **Feasibility spike (no product switch)** — **complete.** Detect output depth, create a D3D11
   device and 10-bit swapchain, present a static gradient, prove fallback and teardown.
2. **Canvas present path** — **implemented in development.** Upload composited frames to the
   swapchain; the canvas is native RGBA16 → 10-bit while egui/guides stay 8-bit and are expanded.
3. **Integration hardening** — **open.** Resize, DPI, multi-monitor, device loss, Remote Desktop.
   DPI/display changes currently fall back to GDI.
4. **Colour management** — **open.** ICC profile reference, working-space conversion, soft
   proofing; these currently force the GDI path.
5. **HDR (optional, later)** — only if there is a concrete workflow and a hardware test target.

## Non-goals

- 16-bit-per-channel scanout.
- HDR/wide-gamut output in the first milestone.
- Linear-light editing or changing working precision.
- Replacing the CPU compositor.

## Risks

- 10-bit panels are uncommon; the improvement is invisible on 8-bit hardware, so this cannot be
  justified purely by a benchmark on the current test machine.
- DXGI path adds driver/OS variance and failure modes (device loss, hybrid GPUs, Remote Desktop).
- egui integration is the least certain part; the spike must prove one of the two overlay options.
- Without colour management, 10-bit output can create a false sense of colour accuracy.

## Relation to the old decision

The v3.7 investigation was correct for its constraint ("preserve the GDI framebuffer and UI"):
under that constraint no path existed. The constraint is now lifted for a new architecture track,
so the decision changes from *rejected* to *deferred with a defined path and gates*.
