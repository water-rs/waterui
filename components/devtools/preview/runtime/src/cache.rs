use std::path::PathBuf;

use waterui_preview_protocol::DylibId;
use waterui_preview_protocol::registry::preview_cache_root_dir;

pub fn preview_dylib_cache_dir() -> PathBuf {
    preview_cache_root_dir().join("dylibs")
}

pub fn preview_dylib_cache_path(id: DylibId) -> PathBuf {
    preview_dylib_cache_dir().join(format!("{id}.dylib"))
}
