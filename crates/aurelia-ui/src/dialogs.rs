use std::path::PathBuf;

pub fn open_project_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Aurelia Project")
        .add_filter("Aurelia Project (*.aur)", &["aur"])
        .add_filter("Legacy Hollow Canvas Project (*.hcv)", &["hcv"])
        .add_filter("Photoshop Document (*.psd) — read-only", &["psd"])
        .pick_file()
}

pub fn save_project_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Save Aurelia Project")
        .add_filter("Aurelia Project (*.aur)", &["aur"])
        .set_file_name("artwork.aur")
        .save_file()
}

pub fn export_png_dialog() -> Option<PathBuf> {
    export_image_dialog("png", "PNG Image (*.png)")
}

/// Save dialog for a flat image export in the given extension (png / jpg / webp).
pub fn export_image_dialog(extension: &str, filter_label: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Export Flat Image")
        .add_filter(filter_label, &[extension])
        .set_file_name(format!("artwork.{}", extension))
        .save_file()
}

pub fn open_image_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open Reference Image")
        .add_filter(
            "Image Files (*.png, *.jpg, *.jpeg, *.webp)",
            &["png", "jpg", "jpeg", "webp"],
        )
        .pick_file()
}

pub fn open_brush_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import Brush")
        .add_filter("Brush Files (*.abr, *.gbr)", &["abr", "gbr"])
        .pick_file()
}

pub fn open_icc_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Assign ICC Profile")
        .add_filter("ICC Profiles (*.icc, *.icm)", &["icc", "icm"])
        .pick_file()
}
