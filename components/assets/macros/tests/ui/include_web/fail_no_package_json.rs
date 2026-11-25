fn main() {
    // `.` resolves to the trybuild scratch crate's root: it exists, and a
    // Cargo project has no package.json.
    let _ = waterui_assets_macros::include_web!(".");
}
