# Clipboard and Adaptive (auto-contrast) layers — v3.7.1

Two features that answer the same class of workflow problem: moving pixels between documents,
and keeping overlay art readable on whatever sits underneath.

## Clipboard: Cut / Copy / Copy Merged / Paste

| Command | Shortcut | Behaviour |
| :--- | :--- | :--- |
| Cut | `Ctrl+X` | Copies the active layer through the selection (or the whole layer), then clears it with the selection used as a coverage mask. One undo step for the clear. |
| Copy | `Ctrl+C` | Copies the active layer through the selection into the internal clipboard. |
| Copy Merged | `Ctrl+Shift+C` | Copies the flattened visible document, with the selection as an alpha mask. |
| Paste | `Ctrl+V` | Pastes as a **new layer** at the copied canvas position; external images are centred on the canvas. |

### What "through the selection" means

The copied region is the selection's pixel bounds (`SelectionMask::bounds`) and each pixel's
alpha is scaled by that pixel's coverage. This matches GIMP/Photoshop feathered copies: a soft
selection produces a soft-edged payload instead of a hard rectangle, and pixels outside the
selection are transparent.

### 16-bit fidelity

The internal clipboard stores the source **storage format**. Copying an RGBA16 layer and
pasting it in Aurelia keeps native `u16` channels; no 8-bit round trip, no colour conversion.
The Windows clipboard copy is a separate, explicitly 8-bit DIBV5 written for interoperability
with other applications. If another application changes the clipboard, Paste prefers that
external image; otherwise the lossless internal payload wins.

### Paste position and layer format

`ClipboardImage` records the canvas-space origin at copy time. Paste creates a canvas-sized
layer with offset zero and writes the region back at that origin, preserving the position the
user copied from. External images (which have no meaningful origin) are centred. Pasting is
undoable through `AddLayerCommand`: undo removes the layer, redo restores it.

### System clipboard encoding

`encode_dibv5` writes a 124-byte `BITMAPV5HEADER` plus bottom-up 32-bit BGRA with `BI_BITFIELDS`
masks and sRGB colour space. `decode_dib` accepts 40/108/124-byte headers, 24- and 32-bit
`BI_RGB`/`BI_BITFIELDS`, top-down or bottom-up rows, and treats an all-zero 32-bit alpha channel
as opaque (some producers leave alpha unused). Malformed or compressed DIBs are rejected, not
guessed at. The conversions are pure functions in `aurelia-core::clipboard` and are unit-tested;
only the Win32 handle plumbing lives in `aurelia-app`.

### Limits

- Copy/Cut require a raster or text layer; groups and adjustment layers have no pixels.
- Layer masks are not part of the payload. A mask is a property of a layer, not its pixels;
  copying the visible RGB is the behaviour both references document.
- The internal clipboard is session-only (one slot), like the system clipboard itself.

## Adaptive layers: readable over anything

A **blend mode** named **Adaptive** (Layers → blend mode) makes a layer choose, per pixel,
between its authored colour and that colour inverted, whichever contrasts more with the
backdrop in luminance:

```
lum_dst = 0.299·R + 0.587·G + 0.114·B          (backdrop)
lum_src = 0.299·r + 0.587·g + 0.114·b          (layer pixel, straight alpha)
keep    = |lum_src − lum_dst| ≥ |(1 − lum_src) − lum_dst|
out_rgb = keep ? src_rgb : 1 − src_rgb          (then normal alpha compositing)
```

Ties keep the authored colour, so a mid-grey layer on a mid-grey backdrop is not arbitrarily
flipped. Fully opaque black artwork therefore reads white on black and stays black on white;
the same layer can sit over a busy photo and remain legible without duplicating it.

### How this compares with Adobe and GIMP

- **Difference / Exclusion** (Photoshop, GIMP) always invert relative to the backdrop. They are
  the closest existing modes and the direct inspiration, but they flip art even when the
  authored colour already contrasts better, and they ignore luminance entirely.
- **Photoshop "Smart Objects"** are not this: they are non-destructive embedded content with
  filters and transforms. They do not change colour based on what is underneath.
- **Photoshop's layer style "Color Overlay"** and **GIMP's "Sample merged"** solve different
  problems (fixed colour, sampling).
- Adaptive is deliberately a **blend mode**, not a hidden automatic layer type: it composes at
  the same point as every other mode, works on mixed 8/16-bit documents, is stored as the layer's
  blend byte (v3 value 13), and is visible in the layer panel. Older builds reading a value 13
  fall back to Normal in the file reader, as they already do for unknown values.

### Limits

- The decision is per pixel and per composite; it is not a "contrast guarantee". A mid-tone
  pixel on a mid-tone backdrop has little contrast in either orientation, and cannot be
  rescued without changing the artwork.
- It adapts to the **composited backdrop below the layer**, not to layers above it. A layer
  above can still cover it.
- It changes colour, not alpha or position; masks, clipping and opacity still apply normally.
- Bicubic/bilinear transforms and filters treat the layer's stored pixels; the adaptation is
  applied when the layer is composited, so previews and exports agree.
