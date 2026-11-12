# PSD test fixtures

These files are used by `tests/psd_import.rs` to exercise real PSD containers:

- `0layers.psd` — composite-only image with RLE image data.
- `1layer.psd` — one raster layer.
- `2layers.psd` — multiple raster layers.
- `empty-group.psd` — real layer section dividers for an empty group.

They are copied from the public `psd-tools` test corpus:
<https://github.com/psd-tools/psd-tools/tree/main/tests/psd_files>

`psd-tools` is MIT-licensed. These files are test-only and are not shipped in Aurelia binaries.
