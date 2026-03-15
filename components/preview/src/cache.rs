use std::path::PathBuf;

use waterkit_fs::WaterFs;
use waterui_preview_protocol::DylibId;

fn water_cache_dir() -> PathBuf {
    if let Some(cache_dir) = std::env::var_os("WATER_CACHE_DIR") {
        return PathBuf::from(cache_dir);
    }

    if let Some(cache_dir) = WaterFs::cache_dir() {
        return cache_dir.join("waterui");
    }

    std::env::temp_dir().join("waterui-cache")
}

pub fn preview_dylib_cache_dir() -> PathBuf {
    water_cache_dir().join("preview").join("dylibs")
}

pub fn preview_dylib_cache_path(id: DylibId) -> PathBuf {
    preview_dylib_cache_dir().join(format!("{}.dylib", id))
}
