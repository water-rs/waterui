//! The stroke record ABI shared by the CPU stroke encoder, the compute
//! stroker's kernels and `build.rs` (water-rs/waterui#2544): plain `const`
//! data only, so `build.rs` can include this file and render the same values
//! into a WGSL prelude. The Rust constants are the only definition.

/// u32 words of one stroke record: six `vec4`s.
pub const RECORD_WORDS: u32 = 24;
// `head.x` bit fields
/// Mask of the piece kind, bits 0-1.
pub const PIECE_MASK: u32 = 0b11;
pub const PIECE_LINE: u32 = 0;
pub const PIECE_CURVE: u32 = 1;
pub const PIECE_POINT_ARC: u32 = 2;
pub const PIECE_JOIN_ONLY: u32 = 3;
/// Shift of the join code, bits 2-4.
pub const JOIN_SHIFT: u32 = 2;
/// Mask of the join code once shifted down.
pub const JOIN_MASK: u32 = 0b111;
/// First piece of a subpath: no join.
pub const JOIN_START: u32 = 0;
/// No join chords: `FS`/`BS` are the previous `FE`/`BE`.
pub const JOIN_SHARED: u32 = 1;
pub const JOIN_BEVEL: u32 = 2;
/// The kernel picks miter or bevel, and the miter side.
pub const JOIN_MITER: u32 = 3;
/// The kernel picks the arc side.
pub const JOIN_ROUND: u32 = 4;
/// Undashed: this piece writes its subpath's start cap.
pub const CAP_START: u32 = 1 << 5;
/// Undashed: this piece writes its subpath's end cap.
pub const CAP_END: u32 = 1 << 6;
/// Closing join of a closed subpath.
pub const CLOSE: u32 = 1 << 7;
/// The final join of a `do_linear` cubic, to its end tangent.
pub const LINEAR_END: u32 = 1 << 8;
// vertex slots per record, in this order
/// Vertex slots per record: `FS`, `BS`, `FE`, `BE`.
pub const SLOTS: u32 = 4;
pub const SLOT_FS: u32 = 0;
pub const SLOT_BS: u32 = 1;
pub const SLOT_FE: u32 = 2;
pub const SLOT_BE: u32 = 3;
/// u32 words of vertex scratch per slot: local xy, device xy.
pub const SLOT_WORDS: u32 = 4;
/// Largest offset chord count of one CURVE piece side.
pub const N_OFF_MAX: u32 = 32;
/// Largest number of Gauss-Legendre sub-intervals of one dashed CURVE piece.
pub const N_LEN_MAX: u32 = 16;
// caps, in the path record's cap word
pub const CAP_BUTT: u32 = 0;
pub const CAP_SQUARE: u32 = 1;
pub const CAP_ROUND: u32 = 2;
// dash fragment flags (staging word 2, bits below `FRAG_RECORD_SHIFT`)
/// The dash covers the record's start vertex.
#[allow(
    dead_code,
    reason = "read by the WGSL prelude build.rs renders, which compiles this file as its own module"
)]
pub const FRAG_JOIN: u32 = 1;
#[allow(
    dead_code,
    reason = "read by the WGSL prelude build.rs renders, which compiles this file as its own module"
)]
pub const FRAG_START_CAP: u32 = 2;
#[allow(
    dead_code,
    reason = "read by the WGSL prelude build.rs renders, which compiles this file as its own module"
)]
pub const FRAG_END_CAP: u32 = 4;
/// Staging word 2 carries the fragment's dashed-record index above the flags,
/// so `dash_emit` and `bin_dashes` find the record without a search.
#[allow(
    dead_code,
    reason = "read by the WGSL prelude build.rs renders, which compiles this file as its own module"
)]
pub const FRAG_RECORD_SHIFT: u32 = 3;
/// u32 words per staged fragment: `t_a`, `t_b`, flags and record, chords.
pub const STAGE_WORDS: u32 = 4;
/// u32 words per compacted fragment: stage index, path-relative output slot.
pub const LIST_WORDS: u32 = 2;
/// Records per arc-length / offset scan block.
pub const SCAN_BLOCK: u32 = 256;
