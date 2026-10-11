//! Engine configuration and reporting types shared by every backend.

/// A byte count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bytes(pub u64);

impl Bytes {
    /// A count of mebibytes.
    #[must_use]
    pub const fn mib(n: u64) -> Self {
        Self(n * 1024 * 1024)
    }
}

/// The engine's memory budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Device-side memory (buffers, textures, the glyph atlas).
    pub gpu: Bytes,
    /// CPU-side resident image and rasterized glyph cache budget.
    /// This bounds cache residency, not total process memory; surface framebuffers
    /// and active retained frame data are not evictable caches.
    pub cpu: Bytes,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            gpu: Bytes::mib(512),
            cpu: Bytes::mib(96),
        }
    }
}

/// System memory pressure reported to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pressure {
    /// Reduce caches where cheap.
    Moderate,
    /// Drop every cache.
    Critical,
}

/// The engine's current memory usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryUsage {
    /// Buffers, textures and the glyph atlas, in bytes.
    pub gpu: Bytes,
    /// CPU-side cache bytes.
    pub cpu: Bytes,
    /// Backdrop-group capture textures, in bytes; included in `gpu`.
    pub backdrop_captures: Bytes,
    /// Texture format of the backdrop captures (`"rgba16float"` normally),
    /// `None` while none is allocated.
    pub backdrop_capture_format: Option<&'static str>,
}
