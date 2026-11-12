# PSD import compatibility

Aurelia imports Photoshop PSD **read-only**. There is no PSD writer and no silent flattening:
unsupported layers or compression methods reject the file with an explicit status message.

## Supported subset

| Feature | Support |
| :--- | :--- |
| PSD version 1 (`8BPS`) | Yes |
| PSB / version 2 | Rejected |
| 8-bit RGB | Yes |
| 16-bit / 32-bit | Rejected |
| Grayscale, indexed, CMYK, Lab, duotone | Rejected |
| Raw and PackBits RLE channel data | Yes |
| ZIP/ZIP-prediction channel data | Rejected |
| Raster layers | Yes |
| Nested groups | Yes |
| Layer masks | Yes, when the mask channel and dimensions are present and consistent |
| Opacity, visibility, clipping flag | Yes |
| Common blend modes (`norm`, `mul`, `scrn`, `over`, `dark`, `lite`, `diff`, `lum`, etc.) | Yes |
| Text layers (`TySh`) | Rejected |
| Adjustment/fill layers (`SoCo`, `GdFl`, curves, levels, etc.) | Rejected |
| Composite image (no layer records) | Imported as `Composite` |
| Layer outside the canvas | Rejected |

Unknown blend modes are rejected rather than silently converted to Normal. Unknown additional
layer blocks are ignored only when they are metadata blocks the importer already recognizes as
safe to skip.

## Limits

- 512 MiB maximum PSD file
- 32,768 pixels per canvas side
- 512 Mi pixels per canvas and aggregate layer pixels
- 4,096 layers
- 1 GiB decoded-byte budget

These are checked before allocating pixel buffers.

## UI

`File → Open` accepts `.psd` with the **read-only** label. Import failures appear in the normal
status/error path. Imported documents are not saved back to PSD; Save writes Aurelia's AUR format.

## Fixtures

`crates/aurelia-io/tests/psd_import.rs` builds in-memory fixtures for:

- composite-only PSD;
- one raster layer and multiple layers;
- nested groups and a layer mask;
- text and adjustment layers;
- malformed signature, truncation and oversized canvas;
- public `load_psd_document` path.

The parser itself is in `crates/aurelia-io/src/psd.rs`.
