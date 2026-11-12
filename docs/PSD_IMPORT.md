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
| Vector masks (`vmsk`/`vscg`) | Rejected |
| Unknown additional-layer metadata blocks | Rejected |
| Composite image (no layer records) | Imported as `Composite` |
| Layer outside the canvas | Rejected |

Unknown blend modes and unknown rendering-affecting metadata blocks are rejected rather than
silently ignored. Only metadata blocks explicitly known to be safe (names, IDs, layer colour and
section/clipping markers) are skipped.

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
- public `load_psd_document` path;
- real PSD containers from the MIT-licensed `psd-tools` corpus:
  - supported: `0layers.psd`, `1layer.psd`, `2layers.psd`
  - expected rejection: `empty-group.psd`, `group.psd`, `layer_mask_data.psd`,
    `blend-and-clipping.psd`, `hidden-groups.psd`, `mask-density-layermask.psd`

Expected-rejection fixtures assert `PsdError::Unsupported`; they are not counted as successful
imports.

The parser itself is in `crates/aurelia-io/src/psd.rs`.
