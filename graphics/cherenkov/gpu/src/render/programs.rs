//! The fixed engine modules' programs as data, shared between the crate
//! and `build.rs` (issue #2726).
//!
//! A program is what one pipeline consumes from a module: a vertex entry
//! with the fragment entry it links against, or a compute entry. DX12
//! passthrough modules hold exactly one entry point, and naga specializes
//! a vertex stage's output linkage against its fragment stage, so DXIL is
//! produced per program and stage. `build.rs` asserts every entry named
//! here exists in its module with the stage named here; a render
//! program's layout is its module's, while a compute program names its
//! own, since the coverage kernels bind two layouts.

use super::bindings::{self, Entry};

/// The bind group layout used by a coverage compute program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ComputeLayout {
    /// The layout used by kernels that do not write dispatch arguments.
    CoverageMain,
    /// The layout used by kernels that write dispatch arguments.
    CoverageArguments,
}

impl ComputeLayout {
    /// Returns the layout's bind groups in group order.
    pub const fn groups(self) -> &'static [&'static [Entry]] {
        match self {
            Self::CoverageMain => bindings::COVERAGE_MAIN_GROUPS,
            Self::CoverageArguments => bindings::COVERAGE_GROUPS,
        }
    }
}

/// One pipeline's entry points.
#[derive(Clone, Copy, Debug)]
pub enum Program {
    /// A render pipeline: the vertex entry and the fragment entry its
    /// output linkage is specialized against.
    Render {
        /// The WGSL vertex entry point.
        vertex: &'static str,
        /// The WGSL fragment entry point.
        fragment: &'static str,
    },
    /// A compute pipeline.
    Compute {
        /// The WGSL compute entry point.
        entry: &'static str,
        /// The pipeline's bind group layout.
        layout: ComputeLayout,
    },
}

const fn render(vertex: &'static str, fragment: &'static str) -> Program {
    Program::Render { vertex, fragment }
}

/// The engine `VARIANT` modules: the painter and the two coverage
/// phases, in `CoveragePass` order.
pub const ENGINE: &[Program; 3] = &[
    render("vs_main", "fs_main"),
    render("vs_opaque", "fs_opaque"),
    render("vs_partial", "fs_partial"),
];

/// The present, mip, resolve and reduce modules' single program.
pub const MAIN: &[Program] = &[render("vs_main", "fs_main")];

/// The external-frame module.
pub const EXTERNAL: &[Program] = &[render("vs_main", "fs_external")];

/// The projective composite module.
pub const PROJECTIVE: &[Program] = &[render("vs_main", "fs_projective")];

/// The silhouette convolution module.
pub const SHADOW: &[Program] = &[render("vs", "fs")];

/// The space conversion module: decode, then encode.
pub const SPACE_CONVERT: &[Program] = &[
    render("vs_main", "fs_decode"),
    render("vs_main", "fs_encode"),
];

const fn kernel(entry: &'static str) -> Program {
    Program::Compute {
        entry,
        layout: ComputeLayout::CoverageMain,
    }
}

const fn argument_kernel(entry: &'static str) -> Program {
    Program::Compute {
        entry,
        layout: ComputeLayout::CoverageArguments,
    }
}

/// The GPU coverage kernels (issues #2543, #3081). The two that write
/// the dispatch-domain argument buffer bind the full group; the others
/// bind the main layout without it.
pub const COVERAGE: &[Program] = &[
    argument_kernel("prepare_cells"),
    argument_kernel("prepare_dispatch"),
    kernel("radix_count"),
    kernel("radix_scan_blocks"),
    kernel("radix_prefix"),
    kernel("radix_scatter"),
    kernel("flatten"),
    kernel("bin"),
    kernel("link_groups"),
    kernel("prefix_tiles"),
    kernel("prefix"),
    kernel("fragment_bins"),
    kernel("integrate_edges"),
    kernel("integrate"),
    kernel("classify"),
    kernel("fill_cells"),
    kernel("emit_counts"),
    // The compute stroker (water-rs/waterui#2544).
    kernel("stroke_vertices"),
    kernel("stroke_emit"),
    kernel("dash_length"),
    kernel("dash_scan_reduce"),
    kernel("dash_scan_blocks"),
    kernel("dash_scan_apply"),
    kernel("dash_fragments"),
    argument_kernel("dash_args"),
    kernel("dash_emit"),
    kernel("bin_strokes"),
    kernel("bin_dashes"),
];
