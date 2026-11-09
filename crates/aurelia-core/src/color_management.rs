//! ICC colour profiles and conversion.
//!
//! `moxcms` is the engine: pure Rust, no C toolchain, and it already supports the 8- and 16-bit
//! RGBA layouts Aurelia uses. This module is the seam that keeps its types out of the document
//! model and gives the rest of the editor one place to attach, validate and convert profiles.
//!
//! The load-bearing rule: **untagged documents are not converted**. Empty profiles preserve the
//! existing sRGB byte-for-byte behaviour, and every new conversion path has to prove that

use std::sync::Arc;

use moxcms::{ColorProfile, Layout, TransformOptions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorProfileError {
    Empty,
    Invalid(String),
    Conversion(String),
}

impl std::fmt::Display for ColorProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "ICC profile is empty"),
            Self::Invalid(reason) => write!(formatter, "invalid ICC profile: {reason}"),
            Self::Conversion(reason) => write!(formatter, "colour conversion failed: {reason}"),
        }
    }
}

impl std::error::Error for ColorProfileError {}

/// A validated ICC profile plus the bytes needed to persist it.
#[derive(Clone)]
pub struct IccProfile {
    bytes: Vec<u8>,
    inner: Arc<ColorProfile>,
}

impl IccProfile {
    /// Parses and validates an ICC profile. Bytes are retained for exact save/round-trip.
    pub fn from_icc(bytes: Vec<u8>) -> Result<Self, ColorProfileError> {
        if bytes.is_empty() {
            return Err(ColorProfileError::Empty);
        }
        let inner = ColorProfile::new_from_slice(&bytes)
            .map_err(|error| ColorProfileError::Invalid(error.to_string()))?;
        Ok(Self {
            bytes,
            inner: Arc::new(inner),
        })
    }

    /// The built-in sRGB profile. Used as the assumed working space for untagged documents.
    pub fn srgb() -> Self {
        Self {
            bytes: Vec::new(),
            inner: Arc::new(ColorProfile::new_srgb()),
        }
    }

    /// Original ICC bytes, empty for the synthetic sRGB profile.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_synthetic(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Converts straight RGBA8 in place.
    pub fn convert_rgba8(
        &self,
        destination: &IccProfile,
        pixels: &mut [u8],
    ) -> Result<(), ColorProfileError> {
        if self.same_as(destination) {
            return Ok(());
        }
        let transform = self
            .inner
            .create_transform_8bit(
                Layout::Rgba,
                &destination.inner,
                Layout::Rgba,
                TransformOptions::default(),
            )
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        let mut converted = vec![0u8; pixels.len()];
        transform
            .transform(pixels, &mut converted)
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        pixels.copy_from_slice(&converted);
        Ok(())
    }

    /// Converts straight RGBA16 in place.
    pub fn convert_rgba16(
        &self,
        destination: &IccProfile,
        pixels: &mut [u16],
    ) -> Result<(), ColorProfileError> {
        if self.same_as(destination) {
            return Ok(());
        }
        let transform = self
            .inner
            .create_transform_16bit(
                Layout::Rgba,
                &destination.inner,
                Layout::Rgba,
                TransformOptions::default(),
            )
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        let mut converted = vec![0u16; pixels.len()];
        transform
            .transform(pixels, &mut converted)
            .map_err(|error| ColorProfileError::Conversion(error.to_string()))?;
        pixels.copy_from_slice(&converted);
        Ok(())
    }

    /// Whether this profile describes the same colour space as `other`, by ICC bytes.
    pub fn same_as(&self, other: &IccProfile) -> bool {
        self.bytes == other.bytes
    }
}

impl std::fmt::Debug for IccProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IccProfile")
            .field("bytes", &self.bytes.len())
            .field("synthetic", &self.is_synthetic())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_garbage_profiles_are_rejected() {
        assert_eq!(IccProfile::from_icc(Vec::new()).unwrap_err(), ColorProfileError::Empty);
        assert!(matches!(
            IccProfile::from_icc(vec![0u8; 128]),
            Err(ColorProfileError::Invalid(_))
        ));
    }

    #[test]
    fn srgb_to_srgb_is_identity_for_8_and_16_bit() {
        let srgb = IccProfile::srgb();
        let mut rgba8 = vec![0u8, 1, 127, 255, 255, 128, 64, 0, 10, 20, 30, 40];
        let original8 = rgba8.clone();
        srgb.convert_rgba8(&srgb, &mut rgba8).expect("convert8");
        assert_eq!(rgba8, original8, "untagged/sRGB path must not move 8-bit bytes");

        let mut rgba16 = vec![0u16, 257, 32768, 65535, 300, 1000, 40000, 12345];
        let original16 = rgba16.clone();
        srgb.convert_rgba16(&srgb, &mut rgba16).expect("convert16");
        assert_eq!(rgba16, original16, "sRGB 16-bit path must not quantise");
    }

    #[test]
    fn alpha_survives_conversion() {
        let srgb = IccProfile::srgb();
        let mut rgba = vec![12u8, 34, 56, 78, 90, 12, 34, 0];
        srgb.convert_rgba8(&srgb, &mut rgba).expect("convert");
        assert_eq!(rgba[3], 78);
        assert_eq!(rgba[7], 0);
    }
}
