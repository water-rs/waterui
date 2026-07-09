//! Filter implementations.
//!
//! Each filter is a pure data struct implementing the [`Filter`](crate::Filter) trait.
//! Filters with `COLOR_ONLY = true` can be automatically fused with adjacent
//! color-only filters for better performance.

mod color;
mod distortion;
mod image;
mod stylize;

pub use color::*;
pub use distortion::*;
pub use image::*;
pub use stylize::*;

#[cfg(test)]
mod shader_contract_tests {
    use std::path::PathBuf;

    /// Every spatial compute shader must declare its workgroup shape via the
    /// `WORKGROUP_X` / `WORKGROUP_Y` tokens. The GPU runtime substitutes them
    /// during shader specialization so the declared shape and the dispatch
    /// math share one source of truth; a hardcoded size would silently
    /// mismatch the dispatch grid.
    #[test]
    fn compute_shaders_declare_workgroup_size_via_tokens() {
        let shader_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
        let mut checked = 0usize;
        let mut pending = vec![shader_root];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).expect("shader directory must be readable") {
                let path = entry
                    .expect("shader directory entry must be readable")
                    .path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "wgsl") {
                    continue;
                }
                let source =
                    std::fs::read_to_string(&path).expect("shader source must be readable");
                if !source.contains("@compute") {
                    continue;
                }
                checked += 1;
                assert!(
                    source.contains("@workgroup_size(WORKGROUP_X, WORKGROUP_Y)"),
                    "compute shader {} must declare @workgroup_size(WORKGROUP_X, WORKGROUP_Y)",
                    path.display()
                );
            }
        }
        assert!(checked > 0, "no compute shaders found under src/shaders");
    }
}
