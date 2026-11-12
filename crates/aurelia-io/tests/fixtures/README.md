# PSD test fixtures

These files are used by `tests/psd_import.rs` to exercise real PSD containers.

## Supported

- `0layers.psd` — composite-only image with RLE image data.
- `1layer.psd` — one raster layer.
- `2layers.psd` — multiple raster layers.

## Expected rejection

These are intentional compatibility-boundary fixtures. The test asserts `PsdError::Unsupported`
and does **not** count them as successful imports.

- `empty-group.psd` — unknown additional-layer metadata (`infx`).
- `group.psd` — adjustment/fill layer.
- `layer_mask_data.psd` — adjustment/fill layer.
- `blend-and-clipping.psd` — adjustment/fill layer.
- `hidden-groups.psd` — adjustment/fill layer.
- `mask-density-layermask.psd` — layer outside the canvas.

They are copied from the public `psd-tools` test corpus:
<https://github.com/psd-tools/psd-tools/tree/main/tests/psd_files>

`psd-tools` is MIT-licensed. These files are test-only and are not shipped in Aurelia binaries.
