//! Installed-font discovery for the text tool.
//!
//! Windows keeps the authoritative list of installed typefaces in
//! `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`, as
//! `"Family (TrueType)" -> "file.ttf"` (or an absolute path for per-user fonts).
//! Reading that key is far cheaper than parsing the name table of every file in the font
//! directory, and it gives the file path the rasterizer needs to load the face.

use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_VALUE_TYPE,
};

const FONT_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";

/// Headroom for a registry value: font file names are short, but user fonts can live in a
/// deep profile path.
const VALUE_BUFFER_BYTES: usize = 4096;

/// One installed face the text tool can use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFont {
    pub family: String,
    pub path: PathBuf,
}

/// Enumerates installed fonts, sorted by family name and de-duplicated.
///
/// Enumeration failures degrade to an empty list: the text tool still works with the
/// built-in font and the manual "Load Custom Font" path.
pub fn installed() -> Vec<SystemFont> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let mut fonts = Vec::new();
    let mut key = HKEY::default();
    let subkey = to_wide(FONT_KEY);
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return fonts;
    }

    let mut index = 0u32;
    loop {
        let mut name = [0u16; 512];
        let mut name_len = name.len() as u32;
        let mut data = [0u8; VALUE_BUFFER_BYTES];
        let mut data_len = data.len() as u32;
        let mut value_type = REG_VALUE_TYPE::default();
        let status = unsafe {
            RegEnumValueW(
                key,
                index,
                Some(windows::core::PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut value_type.0 as *mut u32),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if status == ERROR_NO_MORE_ITEMS || status != ERROR_SUCCESS {
            break;
        }
        index += 1;

        let name = String::from_utf16_lossy(&name[..name_len as usize]);
        let value = utf16_string(&data[..data_len as usize]);
        if let Some(font) = parse_entry(&name, &value, Path::new(&windir)) {
            fonts.push(font);
        }
    }
    unsafe {
        let _ = RegCloseKey(key);
    }

    fonts.sort_by_cached_key(|font| font.family.to_lowercase());
    fonts.dedup_by(|a, b| a.family.eq_ignore_ascii_case(&b.family));
    fonts
}

/// Turns one registry entry into a font, or `None` when it is not a loadable face.
///
/// `name` is `"Family (TrueType)"`, `"Family (OpenType)"` or often just `"Family"`;
/// `data` is a file name relative to `%WINDIR%\Fonts` or an absolute path.
fn parse_entry(name: &str, data: &str, windir: &Path) -> Option<SystemFont> {
    let data = data.trim();
    if data.is_empty() {
        return None;
    }
    let path = Path::new(data);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        windir.join("Fonts").join(path)
    };
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        // .ttc collections are not supported by the rasterizer; listing them would only
        // produce a face that fails to load.
        Some("ttf") | Some("otf") => {}
        _ => return None,
    }
    if !path.exists() {
        return None;
    }
    let family = family_from_value_name(name);
    if family.is_empty() {
        return None;
    }
    Some(SystemFont { family, path })
}

/// `"Arial (TrueType)"` -> `"Arial"`, `"Arial Bold (TrueType)"` -> `"Arial Bold"`.
fn family_from_value_name(name: &str) -> String {
    let family = match name.find(" (") {
        Some(index) => &name[..index],
        None => name,
    };
    family.trim().to_string()
}

fn utf16_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_entries_map_to_family_and_path() {
        let dir = std::env::temp_dir().join(format!("aurelia-fonts-{}", std::process::id()));
        let fonts = dir.join("Fonts");
        std::fs::create_dir_all(&fonts).expect("temp dir");
        let file = fonts.join("Example.ttf");
        std::fs::write(&file, b"not a real font").expect("write");

        let font = parse_entry("Example (TrueType)", "Example.ttf", &dir)
            .expect("relative entry resolves under the font directory");
        assert_eq!(font.family, "Example");
        assert_eq!(font.path, file);

        let absolute = format!("{}", file.display());
        let font = parse_entry("Example Bold (OpenType)", &absolute, &dir)
            .expect("absolute entry is accepted");
        assert_eq!(font.family, "Example Bold");
        assert_eq!(font.path, file);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_and_missing_faces_are_skipped() {
        assert!(parse_entry(
            "Collection (TrueType)",
            "name.ttc",
            Path::new("C:\\Windows")
        )
        .is_none());
        assert!(parse_entry(
            "Missing (TrueType)",
            "missing.ttf",
            Path::new("C:\\Windows")
        )
        .is_none());
        assert!(parse_entry("Blank", "", Path::new("C:\\Windows")).is_none());
        assert_eq!(family_from_value_name("Arial"), "Arial");
        assert_eq!(
            family_from_value_name("Segoe UI Semibold (TrueType)"),
            "Segoe UI Semibold"
        );
    }

    #[test]
    fn installed_fonts_are_discoverable_on_windows() {
        let fonts = installed();
        assert!(
            !fonts.is_empty(),
            "every supported Windows install has at least one TTF/OTF face"
        );
        assert!(
            fonts.len() < 10_000,
            "enumeration must not explode: {} entries",
            fonts.len()
        );
        for font in &fonts {
            assert!(!font.family.is_empty(), "family names are never empty");
            assert!(
                font.path.exists(),
                "listed font {} must exist at {}",
                font.family,
                font.path.display()
            );
        }
        // Families are sorted case-insensitively for the menu.
        let mut sorted = fonts.clone();
        sorted.sort_by_cached_key(|font| font.family.to_lowercase());
        assert_eq!(sorted, fonts, "installed() returns a stable, sorted list");
    }
}
