# RGBA16 viewport presentation — investigation and decision (#44)

Status: **rejected** — the 8-bit presentation path stays. This document records what was
investigated, what was measured, and why a native RGBA16 presentation path is not justified for
v3.7.0. The decision is deliberately conservative: no renderer rewrite, no egui fork, and no
silent colour conversion.

## What the viewport actually is

- `aurelia-render` composites into a **32-bit BGRA** framebuffer (`Vec<u32>` per window pixel)
  that the Win32 host blits with `SetDIBitsToDevice`.
- `egui` 0.28 draws the panels and the Navigator on top. `egui::ColorImage` /
  `TextureHandle` are **RGBA8**, so any texture the UI holds is 8 bits per channel.
- A 16-bit document is composited at working precision (`composite_layers_srgb16`), then
  downconverted for presentation with `srgb16_to_srgb8(value) = (value + 128) / 257`, which is
  rounded, not truncated: an 8-bit document round-trips exactly and every 16-bit value lands on
  its nearest 8-bit code.

So the limitation is in the *last step* (DIB + egui texture), not in storage, compositing,
filters, transform or export — all of which are native 16-bit.

## Paths considered

| Path | What it would take | Verdict |
| :--- | :--- | :--- |
| **GDI 16-bit DIB** | `BITMAPINFO`/`SetDIBitsToDevice` has no 16-bits-per-channel format. 48/64-bpp DIBs are not supported by GDI blits. | Impossible without leaving GDI. |
| **Direct2D** | A `D2D1` render target could draw a higher-precision bitmap, but D2D's bitmap formats stop at 8-bit UNORM for this use; the practical high-precision route is a D3D11 texture and a DXGI swapchain. | New device, swapchain and resize machinery; effectively a renderer rewrite. |
| **D3D11/DXGI swapchain** | Present an R16G16B16A16 texture, keep GDI for the UI, composite the egui layer separately. Needs device-loss handling, DPI/resize integration and a fallback for unsupported hardware. | Out of scope: exceeds the issue's "preserve the existing GDI framebuffer and UI" constraint. |
| **Dither the 16→8 conversion** | Ordered/error-diffusion dither before the DIB blit. Cheap to implement, but it trades banding for visible noise at 100% zoom, changes pixel output (bad for judging colour), and cannot add real precision beyond the panel. | Rejected on fidelity grounds. |
| **Keep 8-bit rounded presentation (status quo)** | No change; document the limitation. | **Chosen.** |

## Measurements

Reference machine: Intel64 Family 6 Model 154 Stepping 3, 12 logical cores, rustc 1.93.1,
release profile. Full numbers and the reproduce commands are in
[PERFORMANCE.md](PERFORMANCE.md).

- 4-layer 1080p composite at 16-bit output: the working-precision path is the same code the
  editor already runs; the extra presentation cost is the 16→8 conversion, which is a single
  multiply/add per channel over the framebuffer.
- Converting 1920×1080 (2.07 Mpx, 8.3 MB of BGRA) with `(v + 128) / 257` is a linear pass over
  the same buffer the blit reads; it is noise next to the composite itself (tens of MPix/s) and
  the 1.6–2.7 ms Navigator downsample recorded in PERFORMANCE.md.
- Frame-time budget: the viewport is already CPU-bound at the composite step; adding a second
  16-bit buffer plus a shader/swapchain path would add a device and a full-frame copy, not
  remove work.

No measurement showed a presentation path that is both cheaper than the status quo and
demonstrably higher fidelity on an SDR 8-bit panel.

## Decision

1. **No native RGBA16 presentation in v3.7.0.** The GDI framebuffer and egui stay as they are.
2. **The limitation is documented**, not hidden: README, ROADMAP and the 16-bit plan all state
   that the viewport presents 16-bit documents at 8 bits per channel, and that this is a display
   limitation rather than a data one.
3. **Native output stays available to non-display consumers.** `Document::composite_layers_srgb16`
   and `composite_layers_into_srgb16` remain the API for export and any future presentation path;
   the conversion used for display is rounded so it does not add bias.
4. **Re-open only with a hardware target.** A future D3D11/DXGI presentation path needs a machine
   with a 10-bit-capable display chain and a measured visible-difference test; without both, the
   issue stays rejected.

## Acceptance mapping

| Issue requirement | Evidence |
| :--- | :--- |
| Preserve GDI framebuffer and UI | No renderer/UI changes; all viewport tests still pass. |
| Keep the current path selectable | It is the only path; no opt-in flag was added because no alternative passed. |
| Investigate without forking egui/rewriting | Paths above were evaluated against the existing architecture. |
| Retain 8-bit fallback | Status quo is the fallback. |
| Document implement-or-reject decision | This document, linked from ROADMAP and the release notes. |
| No silent colour conversion | Display conversion is explicit in the renderer and unchanged. |
