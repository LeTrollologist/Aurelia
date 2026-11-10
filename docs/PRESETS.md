# Brush and gradient presets

Aurelia stores the user's brush and gradient preset library in:

```text
%LOCALAPPDATA%\Aurelia\presets.aurp
```

## Format

`presets.aurp` is a binary file with a four-byte `AURP` header and a little-endian version. The
current format is version 1:

```text
"AURP" | u16 version | u32 brush_count | brushes... | u32 gradient_count | gradients...
```

A brush record contains a bounded JSON `BrushPreset` (name, icon, category, description and
`BrushSettings`) followed by an optional raw stamp:

```text
u32 json_len | JSON | u8 has_stamp
if has_stamp: u32 name_len | name | u32 width | u32 height | u64 alpha_len | alpha
```

A gradient record is just a bounded JSON `GradientPreset` containing the name, description,
gradient type, dither flag and the two endpoint colors.

## Limits

- 512 brush presets and 512 gradient presets
- 4096 bytes per string
- 256 KiB per JSON record
- 8192 pixels per stamp side, 63 Mi pixels total stamp coverage
- 256 MiB total file/decoded budget

Counts and lengths are checked before allocation. Bad magic, malformed UTF-8, truncated records,
unknown versions, oversized stamps and trailing bytes are rejected with an explicit error.

## Durability

Saves use a unique sibling temporary file, `sync_all`, replacement and cleanup on failure. A
failed save leaves the previous `presets.aurp` intact. The file is read at startup after the
preferences file; a missing file keeps the factory library, while a corrupt file is reported in
the preset shelf and does not replace the in-memory defaults.

## Migration

Version 1 is the only accepted format. A future format must add a migration before changing
`PRESET_VERSION`; unknown versions are rejected instead of being reinterpreted. Importing a newer
file is therefore a clear error, never a silent partial load.

## UI

- **Presets** shelf: save a brush preset, import `.abr`/`.gbr` stamps, export/import the library,
  or open the manager.
- **Preset Library** manager: rename/delete brush and gradient presets, then **Save changes**.
- **Gradient tool → Gradient Presets**: apply, save the current gradient, and manage/export the
  library.
