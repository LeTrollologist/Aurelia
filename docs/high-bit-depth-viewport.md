# High-bit-depth viewport presentation — architecture proposal

Status: **proposed** (reassesses the v3.7 "8-bit presentation" decision). This is not a promise of
immediate delivery; it defines the target, the path, and the gates so the gap is tracked as
architecture work rather than closed as out of scope.

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
  → Vec<u32> BGRA8 framebuffer
  → GDI SetDIBitsToDevice
  → egui overlay (RGBA8 textures)
```

GDI blits have no 16-bit-per-channel format, and egui textures are RGBA8. The precision is lost in
the final present step, not in editing.

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

## Dependency: colour management

10-bit output without profile-aware colour management is only half of a professional workflow.
Soft proofing and ICC conversion are a separate milestone (see ROADMAP). The presentation work
should land first because it is the visible gap, but neither alone earns an "end-to-end
high-precision studio" claim.

## Gates

| Gate | Evidence |
| :--- | :--- |
| **Visible improvement** | On a 10-bit display chain, a vertical 16-bit gradient shows no 8-bit banding; a measured step test distinguishes 8-bit and 10-bit presentation. |
| **Fallback is unchanged** | With DXGI unavailable, output is byte-identical to the current GDI path and all viewport digest tests pass. |
| **No startup/perf regression** | Device creation is lazy or bounded; first-frame and resize timings stay within the recorded viewport budget. |
| **Mixed monitors** | A 10-bit primary and 8-bit secondary both present correctly; DPI changes and monitor moves re-evaluate the path. |
| **Remote/VM safety** | Remote Desktop and WARP-only environments fall back without dialogs, crashes or repeated retries. |
| **Colour correctness** | 10-bit values are converted from sRGB-encoded 16-bit data without a linear-light detour; fallback output matches the existing rounded conversion. |

## Phases

1. **Feasibility spike (no product switch):** detect output depth, create a D3D11 device and
   10-bit swapchain, present a static gradient, prove fallback and teardown. No UI integration.
2. **Canvas present path:** upload composited frames to the swapchain; keep egui on the current
   path initially (UI over canvas via DirectComposition or a second pass).
3. **Integration hardening:** resize, DPI, multi-monitor, device loss, Remote Desktop.
4. **Colour management:** ICC profile reference, working-space conversion, soft proofing.
5. **HDR (optional, later):** only if there is a concrete workflow and a hardware test target.

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
