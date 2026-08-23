//! Filter implementations.
//!
//! Each filter is a pure data struct implementing the [`Filter`](crate::Filter) trait.
//! Filters with `COLOR_ONLY = true` can be automatically fused with adjacent
//! color-only filters for better performance.
//!
//! # Declaring one
//!
//! Every built-in filter is written with [`#[derive(Filter)]`](crate::Filter):
//! one attribute names the pass kind and the WGSL source (resolved against the
//! declaring crate's `src/shaders/`), and the fields flatten into the parameter
//! array in declaration order.
//!
//! ```rust
//! use filtrate::{Filter, FilterParam};
//!
//! #[derive(Filter)]
//! #[filter(color_only, shader = "color/adjustment/brightness.wgsl")]
//! struct Brighten<T>(T);
//!
//! #[derive(Filter)]
//! #[filter(spatial, shader = "image/convolution/sobel.wgsl")]
//! struct Edges;
//!
//! assert!(Brighten::<f32>::COLOR_ONLY);
//! assert!(!Edges::COLOR_ONLY);
//! assert_eq!(Brighten(0.2_f32).params(), [0.2]);
//! ```

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
