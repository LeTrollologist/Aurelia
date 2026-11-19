//! Durable, bounded storage for layer-style presets.
//!
//! Deliberately separate from the brush/gradient preset library (`presets.aurp`, `preset_store`):
//! styles and brushes have different schemas and different migration needs, and a corrupt or
//! future-version style library must never make the brush library unreadable.
//!
//! ```text
//! "ASRP" | u16 version | u32 count | entry...
//! entry: u32 name_len | UTF-8 name | u32 json_len | JSON(LayerStyles)
//! ```
//!
//! Every count and length is checked before allocation; the file is written through a unique
//! sibling temporary file and only published after `sync_all`, so a failed save leaves the
//! previous library untouched. Version 1 is the only accepted format.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::layer_style::LayerStyles;

pub const STYLE_PRESET_MAGIC: [u8; 4] = *b"ASRP";
pub const STYLE_PRESET_VERSION: u16 = 1;
pub const MAX_STYLE_PRESETS: u32 = 256;
pub const MAX_STYLE_PRESET_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_STYLE_PRESET_NAME_BYTES: usize = 200;
pub const MAX_STYLE_PRESET_JSON_BYTES: usize = 64 * 1024;

static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum StylePresetError {
    #[error("style preset I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt style preset file: {0}")]
    Corrupt(String),
    #[error("style preset file is too large: {0}")]
    TooLarge(String),
    #[error(
        "unsupported style preset version {0}; this build reads version {STYLE_PRESET_VERSION}"
    )]
    UnsupportedVersion(u16),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StylePreset {
    pub name: String,
    #[serde(default)]
    pub styles: LayerStyles,
}

impl StylePreset {
    pub fn new(name: impl Into<String>, styles: LayerStyles) -> Self {
        Self {
            name: name.into(),
            styles,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StylePresetLibrary {
    pub presets: Vec<StylePreset>,
}

fn validate(library: &StylePresetLibrary) -> Result<Vec<Vec<u8>>, StylePresetError> {
    if library.presets.len() as u32 > MAX_STYLE_PRESETS {
        return Err(StylePresetError::TooLarge(format!(
            "{} presets; maximum {MAX_STYLE_PRESETS}",
            library.presets.len()
        )));
    }
    let mut encoded = Vec::with_capacity(library.presets.len());
    let mut total = 0usize;
    for preset in &library.presets {
        if preset.name.is_empty() {
            return Err(StylePresetError::Corrupt("preset has an empty name".into()));
        }
        if preset.name.len() > MAX_STYLE_PRESET_NAME_BYTES {
            return Err(StylePresetError::TooLarge(format!(
                "preset name is {} bytes; maximum {MAX_STYLE_PRESET_NAME_BYTES}",
                preset.name.len()
            )));
        }
        if preset.styles.is_empty() {
            return Err(StylePresetError::Corrupt(format!(
                "preset '{}' has no effects",
                preset.name
            )));
        }
        let json = serde_json::to_vec(&preset.styles)
            .map_err(|error| StylePresetError::Corrupt(format!("style JSON: {error}")))?;
        if json.len() > MAX_STYLE_PRESET_JSON_BYTES {
            return Err(StylePresetError::TooLarge(format!(
                "style JSON is {} bytes; maximum {MAX_STYLE_PRESET_JSON_BYTES}",
                json.len()
            )));
        }
        total = total
            .checked_add(preset.name.len())
            .and_then(|value| value.checked_add(json.len()))
            .ok_or_else(|| StylePresetError::TooLarge("preset size overflow".into()))?;
        encoded.push(json);
    }
    if total as u64 > MAX_STYLE_PRESET_FILE_BYTES {
        return Err(StylePresetError::TooLarge(format!(
            "style payload is {total} bytes; maximum {MAX_STYLE_PRESET_FILE_BYTES}"
        )));
    }
    Ok(encoded)
}

fn encode(library: &StylePresetLibrary) -> Result<Vec<u8>, StylePresetError> {
    let encoded = validate(library)?;
    let mut out = Vec::new();
    out.extend_from_slice(&STYLE_PRESET_MAGIC);
    out.extend_from_slice(&STYLE_PRESET_VERSION.to_le_bytes());
    out.extend_from_slice(&(library.presets.len() as u32).to_le_bytes());
    for (preset, json) in library.presets.iter().zip(encoded) {
        out.extend_from_slice(&(preset.name.len() as u32).to_le_bytes());
        out.extend_from_slice(preset.name.as_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
    }
    Ok(out)
}

fn decode(bytes: &[u8]) -> Result<StylePresetLibrary, StylePresetError> {
    if bytes.len() as u64 > MAX_STYLE_PRESET_FILE_BYTES {
        return Err(StylePresetError::TooLarge(format!(
            "{} bytes; maximum {MAX_STYLE_PRESET_FILE_BYTES}",
            bytes.len()
        )));
    }
    struct Cursor<'a> {
        data: &'a [u8],
        at: usize,
    }
    impl<'a> Cursor<'a> {
        fn bytes(&mut self, count: usize) -> Result<&'a [u8], StylePresetError> {
            let end = self
                .at
                .checked_add(count)
                .ok_or_else(|| StylePresetError::Corrupt("offset overflow".into()))?;
            let slice = self
                .data
                .get(self.at..end)
                .ok_or_else(|| StylePresetError::Corrupt("truncated preset file".into()))?;
            self.at = end;
            Ok(slice)
        }
        fn u16(&mut self) -> Result<u16, StylePresetError> {
            let slice = self.bytes(2)?;
            Ok(u16::from_le_bytes([slice[0], slice[1]]))
        }
        fn u32(&mut self) -> Result<u32, StylePresetError> {
            let slice = self.bytes(4)?;
            Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
        }
        fn string(&mut self, max: usize) -> Result<String, StylePresetError> {
            let length = self.u32()? as usize;
            if length > max {
                return Err(StylePresetError::TooLarge(format!(
                    "{length}-byte string; maximum {max}"
                )));
            }
            let bytes = self.bytes(length)?;
            String::from_utf8(bytes.to_vec())
                .map_err(|_| StylePresetError::Corrupt("preset string is not UTF-8".into()))
        }
    }

    let mut cursor = Cursor { data: bytes, at: 0 };
    if cursor.bytes(4)? != STYLE_PRESET_MAGIC {
        return Err(StylePresetError::Corrupt("bad ASRP signature".into()));
    }
    let version = cursor.u16()?;
    if version != STYLE_PRESET_VERSION {
        return Err(StylePresetError::UnsupportedVersion(version));
    }
    let count = cursor.u32()?;
    if count > MAX_STYLE_PRESETS {
        return Err(StylePresetError::TooLarge(format!(
            "{count} presets; maximum {MAX_STYLE_PRESETS}"
        )));
    }
    let mut presets = Vec::new();
    presets
        .try_reserve(count as usize)
        .map_err(|_| StylePresetError::TooLarge("preset list cannot be allocated".into()))?;
    for _ in 0..count {
        let name = cursor.string(MAX_STYLE_PRESET_NAME_BYTES)?;
        let json_len = cursor.u32()? as usize;
        if json_len > MAX_STYLE_PRESET_JSON_BYTES {
            return Err(StylePresetError::TooLarge(format!(
                "style JSON is {json_len} bytes; maximum {MAX_STYLE_PRESET_JSON_BYTES}"
            )));
        }
        let json = cursor.bytes(json_len)?;
        let styles: LayerStyles = serde_json::from_slice(json)
            .map_err(|error| StylePresetError::Corrupt(format!("style JSON: {error}")))?;
        if styles.is_empty() {
            return Err(StylePresetError::Corrupt(format!(
                "preset '{name}' has no effects"
            )));
        }
        presets.push(StylePreset { name, styles });
    }
    Ok(StylePresetLibrary { presets })
}

/// Loads a style library. A missing file is an empty library, not an error.
pub fn load_style_library(path: &Path) -> Result<StylePresetLibrary, StylePresetError> {
    match std::fs::read(path) {
        Ok(bytes) => decode(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(StylePresetLibrary::default())
        }
        Err(error) => Err(StylePresetError::Io(error)),
    }
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{sequence}.tmp", std::process::id()))
}

/// Writes a style library atomically: unique sibling temp, `sync_all`, then rename.
pub fn save_style_library(
    library: &StylePresetLibrary,
    path: &Path,
) -> Result<(), StylePresetError> {
    let bytes = encode(library)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let temp = unique_temp_path(path);
    let result = (|| -> Result<(), StylePresetError> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::layer_style::InnerGlowStyle;

    fn fixture() -> StylePresetLibrary {
        let mut styles = LayerStyles::default();
        styles.inner_glow = Some(InnerGlowStyle {
            radius: 4.0,
            color: Color::WHITE,
            opacity: 0.5,
        });
        StylePresetLibrary {
            presets: vec![StylePreset::new("Glow", styles)],
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "aurelia-styles-{tag}-{}-{}.aurp",
            std::process::id(),
            SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn round_trip() {
        let path = temp_path("roundtrip");
        let library = fixture();
        save_style_library(&library, &path).expect("save");
        let loaded = load_style_library(&path).expect("load");
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded, library);
    }

    #[test]
    fn missing_file_is_an_empty_library() {
        let path = temp_path("missing");
        let loaded = load_style_library(&path).expect("missing file loads empty");
        assert!(loaded.presets.is_empty());
    }

    #[test]
    fn unknown_version_and_garbage_are_rejected() {
        let path = temp_path("garbage");
        std::fs::write(&path, b"ASRP\x02\x00\x00\x00\x00\x00").expect("write");
        match load_style_library(&path) {
            Err(StylePresetError::UnsupportedVersion(2)) => {}
            other => panic!("expected UnsupportedVersion(2), got {other:?}"),
        }
        std::fs::write(&path, b"nope").expect("write");
        assert!(matches!(
            load_style_library(&path),
            Err(StylePresetError::Corrupt(_))
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn oversized_counts_and_names_are_rejected_before_allocating() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&STYLE_PRESET_MAGIC);
        bytes.extend_from_slice(&STYLE_PRESET_VERSION.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(decode(&bytes), Err(StylePresetError::TooLarge(_))));

        let mut library = fixture();
        library.presets[0].name = "x".repeat(MAX_STYLE_PRESET_NAME_BYTES + 1);
        assert!(matches!(
            encode(&library),
            Err(StylePresetError::TooLarge(_))
        ));

        let mut empty = fixture();
        empty.presets[0].styles = LayerStyles::default();
        assert!(matches!(encode(&empty), Err(StylePresetError::Corrupt(_))));
    }

    #[test]
    fn a_failed_save_keeps_the_previous_library() {
        let directory = temp_path("atomic-dir");
        std::fs::create_dir_all(&directory).expect("dir");
        let path = directory.join("styles.aurp");
        save_style_library(&fixture(), &path).expect("initial save");
        let original = std::fs::read(&path).expect("read");

        let mut invalid = fixture();
        invalid.presets[0].name.clear();
        assert!(save_style_library(&invalid, &path).is_err());
        assert_eq!(std::fs::read(&path).expect("read"), original);
        let leftovers = std::fs::read_dir(&directory)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "a failed save must remove its temp file");
        let _ = std::fs::remove_dir_all(&directory);
    }
}
