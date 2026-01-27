use std::path::PathBuf;

use waterui_preview_protocol::DylibId;

fn water_cache_dir() -> PathBuf {
    dirs::home_dir()
        .expect("preview requires a home directory for ~/.water cache")
        .join(".water")
        .join("cache")
}

pub fn preview_dylib_cache_dir() -> PathBuf {
    water_cache_dir().join("preview").join("dylibs")
}

pub fn preview_dylib_cache_path(id: DylibId) -> PathBuf {
    preview_dylib_cache_dir().join(format!("{}.dylib", id))
}

