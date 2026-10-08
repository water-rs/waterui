//! The backdrop union member cap, shared engine-free.

/// The maximum member count of a `BackdropUnion` group — the file is
/// pulled into the oracle and the build script by `#[path]`, so the
/// name stays plain text rather than an intra-doc link. The GPU's
/// `UNION_MAX_MEMBERS` WGSL constant is emitted from
/// this value by `gpu/build.rs`, and the oracle reads the same file —
/// the engines share the single definition without depending on this
/// crate.
pub const UNION_MAX_MEMBERS: u32 = 32;
