# Brush stamp import (`.abr` / `.gbr`)

Aurelia can import sampled brush stamps from GIMP and Photoshop brush files. The imported stamp
becomes a normal brush preset in the **Imported** category and paints with the current foreground
colour.

## Formats

| Format | Support |
| :--- | :--- |
| GIMP `.gbr` | Full: 1-byte grayscale or 4-byte RGBA pixels, NUL-terminated name, dimensions capped at 4096. Alpha is the grayscale byte (bpp 1) or the fourth byte (bpp 4). |
| Photoshop `.abr` v1/v2 | Legacy sampled entries: 8/16-bit raw or RLE tips, v2 Unicode names. |
| Photoshop `.abr` v6/v7/v9/v10 | Raw sampled tips are found by scanning for the shared bitmap header inside the container. RLE tips and computed brushes are not imported. |

Malformed files are rejected with a structured error; no decoder path panics on truncation,
oversized dimensions, invalid magic, unterminated names or truncated pixels.

## Behaviour

- The stamp keeps its aspect ratio: the larger dimension spans the brush diameter, the smaller
  dimension is proportionally narrower.
- The stamp multiplies the brush opacity; wet-edge/velocity/tilt dynamics still apply around it.
- Imported presets are **session-only**. Brush preferences do not embed bitmaps, so the `stamp`
  field is skipped during serialization rather than silently degrading the preset to a procedural
  round brush on the next launch. Re-import after restarting until a persistent brush library
  exists.
- `Space`/spray tools ignore the stamp (they have their own stochastic tip).
- Files are read through `assets::AssetSource`; the decoder receives bytes, not paths.

## UI

Right dock → Presets → **＋ Import brush**, then choose an `.abr` or `.gbr`. The status bar reports
how many stamps were imported; the last one is selected immediately.
