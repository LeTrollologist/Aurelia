# Project format (AUR)

AUR is Aurelia's binary project format. Saving always writes **v3**; the reader accepts v1 and v2
for compatibility and legacy Hollow Canvas `.hcv` v2–v5.

## Container

```text
magic            4 bytes   "AUR\x01" / "AUR\x02" / "AUR\x03"
width, height    u32 LE each
active_layer_id  u64 LE
background       u8
is_transparent   u8
theme            u8
document_format  u8        v3 only: 0 = sRGB8, 1 = sRGB16
layer_count      u32 LE
```

Each layer then carries: id, UTF-8 name, kind, adjustment/text config, parent id, flags,
opacity, blend mode, offsets, an optional v3 format tag, `comp_len`, a raw-deflate payload, and
an optional mask section. The preview thumbnail is a PNG appended after the layers and is never
required for loading.

## Versions

| Version | Adds | Reader behavior |
| :--- | :--- | :--- |
| v1 | Initial `.aur` container (HCV v5 body layout) | read-only |
| v2 | Per-layer 8-bit masks | read-only |
| v3 | Per-document and per-layer pixel-format tags; 16-bit layers | written |

## The document format tag

The `document_format` byte is **the document's storage format for new layers**, not a summary of
what the layers happen to hold:

- The writer emits `doc.pixel_format`. It used to derive the byte from "does any layer hold
  16-bit channels", which meant deleting the last 16-bit layer silently turned a 16-bit document
  into an 8-bit one, and the next new layer was created 8-bit.
- A layer's own tag may still differ from the document's (`0` = inherit, `1` = sRGB8,
  `2` = sRGB16). Mixed documents are therefore representable, loadable, and save unchanged; the
  editor simply does not create them.
- An unknown document tag is a hard error, never a fallback.

## Limits and validation

- `width * height * 8` must be ≤ 2 GiB; zero dimensions are rejected.
- A layer's deflate output is bounded to the exact expected payload (+1 byte probe); a
  high-ratio stream is rejected before it can allocate the expansion.
- Truncated or malformed files return `CorruptData`; the reader does not guess a format.
- 16-bit layers round-trip byte-exactly, and native 16-bit compositing output is available
  through `Document::composite_layers_srgb16` (#15).

## Editor state section (`AED1`)

AUR v3 optionally appends an editor-state section **after** the preview PNG. The preview and this
section are outside the pixel/layer payload, so old readers stop after the layers and ignore them.

```text
magic           4 bytes   "AED1"
payload_len     u32 LE    <= 1 MiB
payload:
  version       u8        1
  guide_count   u32 LE    <= 4096
  guides        guide_count * (vertical u8, position f32 LE)
  grid_origin   f32 LE x, f32 LE y
  ruler_flag    u8        0 = absent, 1 = present
  ruler         if present: origin x/y f32 LE, angle_rad f32 LE, length f32 LE
```

Semantics:

- `vertical = 1` is a vertical guide; `0` is horizontal. Positions are canvas pixels.
- The section is optional and additive. Files written before it load with an empty guide list,
  zero grid origin and no straight-edge ruler.
- The reader skips the preview by its length, then reads `AED1` if present. A missing section,
  trailing EOF, unknown magic or unknown payload version is treated as **no editor state**; it
  is not a corrupt-project error. A section longer than 1 MiB, more than 4096 guides, or
  non-finite coordinates is ignored and defaults are used.
- The section does not affect pixel/layer round-trip guarantees and does not bump the container
  version. Older readers that ignore trailing bytes remain compatible.

## Layer styles section (`ASTY`, v3.8.0)

After the editor section, AUR v3 may append a per-layer style section. It is only written when at
least one layer has styles:

```
magic           4 bytes   "ASTY"
payload_len     u32 LE    bytes after this field (1 .. 1 MiB)
payload:
  version       u8        1
  count         u32 LE    layers with styles
  each entry:
    layer_id    u64 LE
    flags       u8        bit0 stroke, bit1 drop shadow, bit2 outer glow
    stroke:     width f32, opacity f32, r/g/b/a u8   (if bit0)
    shadow:     offset x/y f32, blur f32, opacity f32, r/g/b/a u8 (if bit1)
    glow:       radius f32, opacity f32, r/g/b/a u8   (if bit2)
```

Semantics:

- Optional and additive: a file without the section loads with empty styles, and a file saved by
  an older build simply ends earlier. The container version is unchanged.
- The section is ignored when it is absent, truncated, longer than 1 MiB, has an unknown version,
  a count above 100 000 or larger than the payload can hold (9 bytes per entry), or non-finite
  floats. A damaged optional section never fails the load.
- Opacity is clamped to `0..=1` and width/radius/blur to `>= 0` on read; unknown flag bits are
  skipped by their field layout only through known versions, so a future bit requires a version
  bump.
- Styles never change layer pixels or the layer record; they are a composite-time property.

## Colour profile section (`AICC`, v3.13)

After the style section an AUR v3 file may append the document's ICC profile:

```
magic           4 bytes   "AICC"
length          u32 LE    ICC bytes (1 .. 4 MiB)
payload         length bytes, an ICC profile
```

The trailing sections are a sequence of length-prefixed records (`ASTY`, `AICC`, future ones).
Readers walk the tail by magic+length, ignore unknown records, and stop cleanly on truncation; a
missing profile means the document is untagged sRGB. A profile that fails validation is ignored
rather than failing the document. The container version is unchanged: files without these sections
load exactly as before.

## Round-trip policy

`load → save → load` must preserve layer order, names, masks, blend modes, offsets, adjustment
settings, layer styles and 8/16-bit channel data. Tests under `crates/aurelia-io/src/tests.rs`,
`crates/aurelia-io/tests/layer_styles_roundtrip.rs` and
`crates/aurelia-io/tests/persistence_hardening.rs` enforce this for the supported versions; the
hardening suite also proves every truncation and optional-section mutation loads without panicking.
A permanent fixture corpus for historical files remains follow-up work in
[RELEASE_PROCESS.md](../RELEASE_PROCESS.md).

## Recovery snapshots (autosave, not part of AUR)

Crash-recovery files live under `%LOCALAPPDATA%\Aurelia\autosave` and are ordinary AUR v3 files.
Their names are `<stem>-<path-hash>-autosave.aur`, where the hash is FNV-1a over the lowercased
full document path; this keeps two projects with the same stem from sharing a snapshot. Startup
offers snapshots newest-first, consumes the restored file, and deletes declined ones. Saving a
document also removes the legacy `<stem>-autosave.aur` name written by v3.9 and earlier.
