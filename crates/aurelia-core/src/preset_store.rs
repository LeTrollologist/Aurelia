//! Durable, bounded storage for user brush and gradient presets.
//!
//! The file is deliberately small and self-describing:
//!
//! ```text
//! "AURP" | u16 version | u32 brush_count | brushes... | u32 gradient_count | gradients...
//! brush:   u32 json_len | JSON(BrushPreset) | u8 has_stamp | stamp...
//! stamp:   u32 name_len | UTF-8 name | u32 width | u32 height | u64 alpha_len | alpha
//! gradient:u32 json_len | JSON(GradientPreset)
//! ```
//!
//! Every count and length is checked before allocation; the aggregate decoded budget is the
//! same order as the brush-import budget. A failed save leaves the previous file untouched.
//!
//! Version 1 is the only accepted format. A future version must add an explicit migration
//! before `load_library` accepts it; unknown versions are rejected, never reinterpreted.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::brush::{BrushPreset, GradientPreset};
use crate::brush_import::BrushStamp;

pub const PRESET_MAGIC: [u8; 4] = *b"AURP";
pub const PRESET_VERSION: u16 = 1;
pub const MAX_PRESETS: u32 = 512;
pub const MAX_PRESET_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_STAMP_PIXELS: u64 = 63 * 1024 * 1024;
pub const MAX_STAMP_SIDE: u32 = 8192;
pub const MAX_STRING_BYTES: usize = 4096;
pub const MAX_JSON_BYTES: usize = 256 * 1024;

static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum PresetError {
    #[error("preset I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt preset file: {0}")]
    Corrupt(String),
    #[error("preset file is too large: {0}")]
    TooLarge(String),
    #[error("unsupported preset version {0}; this build reads version {PRESET_VERSION}")]
    UnsupportedVersion(u16),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresetLibrary {
    pub brushes: Vec<BrushPreset>,
    pub gradients: Vec<GradientPreset>,
}

impl Default for PresetLibrary {
    fn default() -> Self {
        Self {
            brushes: BrushPreset::default_library(),
            gradients: GradientPreset::default_library(),
        }
    }
}

fn validate_string(value: &str, field: &str) -> Result<(), PresetError> {
    if value.len() > MAX_STRING_BYTES {
        return Err(PresetError::TooLarge(format!(
            "{field} is {} bytes; maximum {MAX_STRING_BYTES}",
            value.len()
        )));
    }
    Ok(())
}

fn checked_stamp(stamp: &BrushStamp, budget: &mut u64) -> Result<(), PresetError> {
    if stamp.width == 0 || stamp.height == 0 {
        return Err(PresetError::Corrupt("stamp has a zero dimension".into()));
    }
    if stamp.width > MAX_STAMP_SIDE || stamp.height > MAX_STAMP_SIDE {
        return Err(PresetError::TooLarge(format!(
            "stamp {}x{} exceeds the {MAX_STAMP_SIDE}px side limit",
            stamp.width, stamp.height
        )));
    }
    let pixels = stamp.width as u64 * stamp.height as u64;
    if pixels > MAX_STAMP_PIXELS {
        return Err(PresetError::TooLarge(format!(
            "stamp {}x{} exceeds the {MAX_STAMP_PIXELS} pixel budget",
            stamp.width, stamp.height
        )));
    }
    if stamp.alpha.len() as u64 != pixels {
        return Err(PresetError::Corrupt(format!(
            "stamp {}x{} has {} coverage bytes, expected {pixels}",
            stamp.width,
            stamp.height,
            stamp.alpha.len()
        )));
    }
    *budget = budget
        .checked_sub(pixels)
        .ok_or_else(|| PresetError::TooLarge("aggregate stamp budget exceeded".into()))?;
    Ok(())
}

fn validate_library(library: &PresetLibrary) -> Result<(), PresetError> {
    if library.brushes.len() as u32 > MAX_PRESETS || library.gradients.len() as u32 > MAX_PRESETS {
        return Err(PresetError::TooLarge(format!(
            "more than {MAX_PRESETS} presets"
        )));
    }
    let mut stamp_budget = MAX_STAMP_PIXELS;
    for brush in &library.brushes {
        validate_string(&brush.name, "brush name")?;
        validate_string(&brush.icon, "brush icon")?;
        validate_string(&brush.category, "brush category")?;
        validate_string(&brush.description, "brush description")?;
        if let Some(stamp) = &brush.settings.stamp {
            validate_string(&stamp.name, "stamp name")?;
            checked_stamp(stamp, &mut stamp_budget)?;
        }
    }
    for gradient in &library.gradients {
        validate_string(&gradient.name, "gradient name")?;
        validate_string(&gradient.description, "gradient description")?;
    }
    Ok(())
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_string(out: &mut Vec<u8>, value: &str) {
    write_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

fn push_json(out: &mut Vec<u8>, value: &[u8]) -> Result<(), PresetError> {
    if value.len() > MAX_JSON_BYTES {
        return Err(PresetError::TooLarge(format!(
            "preset JSON is {} bytes; maximum {MAX_JSON_BYTES}",
            value.len()
        )));
    }
    write_u32(out, value.len() as u32);
    out.extend_from_slice(value);
    Ok(())
}

/// Serialize a library into a bounded byte vector.
pub fn encode_library(library: &PresetLibrary) -> Result<Vec<u8>, PresetError> {
    validate_library(library)?;
    let mut out = Vec::with_capacity(64 * 1024);
    out.extend_from_slice(&PRESET_MAGIC);
    out.extend_from_slice(&PRESET_VERSION.to_le_bytes());
    write_u32(&mut out, library.brushes.len() as u32);
    for brush in &library.brushes {
        let json = serde_json::to_vec(brush)
            .map_err(|error| PresetError::Corrupt(format!("brush JSON: {error}")))?;
        push_json(&mut out, &json)?;
        if let Some(stamp) = &brush.settings.stamp {
            out.push(1);
            push_string(&mut out, &stamp.name);
            write_u32(&mut out, stamp.width);
            write_u32(&mut out, stamp.height);
            write_u64(&mut out, stamp.alpha.len() as u64);
            out.extend_from_slice(&stamp.alpha);
        } else {
            out.push(0);
        }
        if out.len() as u64 > MAX_PRESET_BYTES {
            return Err(PresetError::TooLarge(
                "encoded preset file exceeded budget".into(),
            ));
        }
    }
    write_u32(&mut out, library.gradients.len() as u32);
    for gradient in &library.gradients {
        let json = serde_json::to_vec(gradient)
            .map_err(|error| PresetError::Corrupt(format!("gradient JSON: {error}")))?;
        push_json(&mut out, &json)?;
    }
    if out.len() as u64 > MAX_PRESET_BYTES {
        return Err(PresetError::TooLarge(
            "encoded preset file exceeded budget".into(),
        ));
    }
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    decoded: u64,
    budget: u64,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            decoded: 0,
            budget: MAX_PRESET_BYTES,
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], PresetError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| PresetError::Corrupt("offset overflow".into()))?;
        if end > self.data.len() {
            return Err(PresetError::Corrupt("truncated preset file".into()));
        }
        self.decoded = self
            .decoded
            .checked_add(count as u64)
            .ok_or_else(|| PresetError::TooLarge("decoded budget overflow".into()))?;
        if self.decoded > self.budget {
            return Err(PresetError::TooLarge(
                "decoded preset budget exceeded".into(),
            ));
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, PresetError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PresetError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, PresetError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, PresetError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn string(&mut self, field: &str) -> Result<String, PresetError> {
        let len = self.u32()? as usize;
        if len > MAX_STRING_BYTES {
            return Err(PresetError::TooLarge(format!(
                "{field} length {len} exceeds {MAX_STRING_BYTES}"
            )));
        }
        String::from_utf8(self.take(len)?.to_vec())
            .map_err(|_| PresetError::Corrupt(format!("{field} is not UTF-8")))
    }

    fn json(&mut self) -> Result<&'a [u8], PresetError> {
        let len = self.u32()? as usize;
        if len > MAX_JSON_BYTES {
            return Err(PresetError::TooLarge(format!(
                "preset JSON length {len} exceeds {MAX_JSON_BYTES}"
            )));
        }
        self.take(len)
    }

    fn finish(&self) -> Result<(), PresetError> {
        if self.position != self.data.len() {
            return Err(PresetError::Corrupt(format!(
                "{} trailing bytes",
                self.data.len() - self.position
            )));
        }
        Ok(())
    }
}

/// Decode a library from a bounded byte slice.
pub fn decode_library(data: &[u8]) -> Result<PresetLibrary, PresetError> {
    if data.len() as u64 > MAX_PRESET_BYTES {
        return Err(PresetError::TooLarge("preset file exceeds budget".into()));
    }
    let mut reader = Reader::new(data);
    if reader.take(4)? != PRESET_MAGIC {
        return Err(PresetError::Corrupt("bad AURP magic".into()));
    }
    let version = reader.u16()?;
    if version != PRESET_VERSION {
        return Err(PresetError::UnsupportedVersion(version));
    }
    let brush_count = reader.u32()?;
    if brush_count > MAX_PRESETS {
        return Err(PresetError::TooLarge(format!(
            "brush count {brush_count} exceeds {MAX_PRESETS}"
        )));
    }
    let mut brushes = Vec::with_capacity(brush_count as usize);
    let mut stamp_budget = MAX_STAMP_PIXELS;
    for _ in 0..brush_count {
        let json = reader.json()?;
        let mut brush: BrushPreset = serde_json::from_slice(json)
            .map_err(|error| PresetError::Corrupt(format!("brush JSON: {error}")))?;
        if reader.u8()? != 0 {
            let name = reader.string("stamp name")?;
            let width = reader.u32()?;
            let height = reader.u32()?;
            if width == 0 || height == 0 || width > MAX_STAMP_SIDE || height > MAX_STAMP_SIDE {
                return Err(PresetError::Corrupt("invalid stamp dimensions".into()));
            }
            let pixels = width as u64 * height as u64;
            if pixels > MAX_STAMP_PIXELS {
                return Err(PresetError::TooLarge("stamp exceeds pixel budget".into()));
            }
            let alpha_len = reader.u64()?;
            if alpha_len != pixels {
                return Err(PresetError::Corrupt(format!(
                    "stamp coverage length {alpha_len} does not match {pixels} pixels"
                )));
            }
            if pixels > stamp_budget {
                return Err(PresetError::TooLarge(
                    "aggregate stamp budget exceeded".into(),
                ));
            }
            stamp_budget -= pixels;
            let alpha = reader.take(pixels as usize)?.to_vec();
            brush.settings.stamp = Some(BrushStamp {
                name,
                width,
                height,
                alpha,
            });
        }
        brushes.push(brush);
    }
    let gradient_count = reader.u32()?;
    if gradient_count > MAX_PRESETS {
        return Err(PresetError::TooLarge(format!(
            "gradient count {gradient_count} exceeds {MAX_PRESETS}"
        )));
    }
    let mut gradients = Vec::with_capacity(gradient_count as usize);
    for _ in 0..gradient_count {
        let json = reader.json()?;
        gradients.push(
            serde_json::from_slice(json)
                .map_err(|error| PresetError::Corrupt(format!("gradient JSON: {error}")))?,
        );
    }
    reader.finish()?;
    Ok(PresetLibrary { brushes, gradients })
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("presets.aurp");
    parent.join(format!(
        ".{name}.{}.{}.{}.tmp",
        std::process::id(),
        nanos,
        sequence
    ))
}

/// Atomically write a library. A failed write leaves the previous file intact.
pub fn save_library(path: &Path, library: &PresetLibrary) -> Result<(), PresetError> {
    let data = encode_library(library)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = unique_temp_path(path);
    let result = (|| -> Result<(), PresetError> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        if let Some(parent) = path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Read and validate a library from disk.
pub fn load_library(path: &Path) -> Result<PresetLibrary, PresetError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_PRESET_BYTES {
        return Err(PresetError::TooLarge(format!(
            "file is {} bytes; maximum {MAX_PRESET_BYTES}",
            metadata.len()
        )));
    }
    decode_library(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::BrushSettings;
    use crate::color::Color;

    fn sample_library() -> PresetLibrary {
        let mut stamps = BrushPreset {
            name: "Stamp".into(),
            icon: "S".into(),
            category: "User".into(),
            description: "test".into(),
            settings: BrushSettings::default(),
        };
        stamps.settings.stamp = Some(BrushStamp {
            name: "stamp".into(),
            width: 2,
            height: 2,
            alpha: vec![0, 64, 128, 255],
        });
        PresetLibrary {
            brushes: vec![stamps],
            gradients: vec![GradientPreset {
                name: "Gradient".into(),
                description: "test".into(),
                gradient_type: crate::brush::GradientType::Radial,
                dither: false,
                primary_color: Color::WHITE,
                secondary_color: Color::BLACK,
            }],
        }
    }

    #[test]
    fn library_round_trips_through_the_binary_format() {
        let library = sample_library();
        let encoded = encode_library(&library).expect("encode");
        assert_eq!(&encoded[..4], &PRESET_MAGIC);
        assert_eq!(decode_library(&encoded).expect("decode"), library);
    }

    #[test]
    fn bad_magic_and_future_version_are_rejected() {
        let mut encoded = encode_library(&sample_library()).expect("encode");
        encoded[0] = b'X';
        assert!(matches!(
            decode_library(&encoded),
            Err(PresetError::Corrupt(_))
        ));

        let mut encoded = encode_library(&sample_library()).expect("encode");
        encoded[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert!(matches!(
            decode_library(&encoded),
            Err(PresetError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn oversized_stamp_budget_is_rejected_before_allocating() {
        let mut library = sample_library();
        let stamp = library.brushes[0].settings.stamp.as_mut().expect("stamp");
        stamp.width = MAX_STAMP_SIDE;
        stamp.height = MAX_STAMP_SIDE;
        stamp.alpha = Vec::new();
        assert!(matches!(
            encode_library(&library),
            Err(PresetError::TooLarge(_))
        ));
    }

    fn test_directory(label: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("preset-store-tests");
        std::fs::create_dir_all(&base).expect("test base directory");
        base.join(format!(
            "{label}-{}-{}",
            std::process::id(),
            SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn library_survives_save_and_reload_from_disk() {
        let directory = test_directory("reload");
        std::fs::create_dir_all(&directory).expect("directory");
        let path = directory.join("presets.aurp");
        let library = sample_library();
        save_library(&path, &library).expect("save");
        let reloaded = load_library(&path).expect("reload");
        assert_eq!(reloaded, library);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn failed_save_preserves_the_previous_file() {
        let directory = test_directory("failed-save");
        std::fs::create_dir_all(&directory).expect("directory");
        let path = directory.join("presets.aurp");
        save_library(&path, &sample_library()).expect("first save");
        let before = std::fs::read(&path).expect("read");

        let mut bad = sample_library();
        bad.brushes[0].name = "x".repeat(MAX_STRING_BYTES + 1);
        assert!(save_library(&path, &bad).is_err());
        assert_eq!(std::fs::read(&path).expect("read"), before);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
