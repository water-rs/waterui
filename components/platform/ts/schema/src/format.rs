//! Wire-format constants shared by the const encoder and the runtime decoder.
//!
//! Every byte a payload may contain is non-zero. The `water` CLI recovers a
//! `#[used] static` from an artifact by reading its section from the symbol
//! address and cutting at the first NUL — Mach-O symbols carry no size — so a
//! NUL inside the payload would truncate it. The encoder asserts that
//! invariant on every byte it emits, and the static appends exactly one NUL as
//! its terminator.

/// Version of the encoded payload format, the first byte of every payload.
///
/// A decoder that meets a version it does not implement fails rather than
/// guessing: the schema is a compatibility contract, and a half-understood
/// contract is worse than none.
pub const FORMAT_VERSION: u8 = 2;

/// The most elements a fixed-length array may project as a tuple type.
///
/// `[T; N]` becomes the TypeScript tuple `[T, T, …]` with one element written
/// out per slot, so a large `N` produces a type nobody can read and a `.d.ts`
/// nobody wants to compile. Past this bound the answer is `Vec<T>`, which
/// projects as `T[]` and carries no length in its type. The const encoder
/// asserts it, so an array too long for the projection fails the build rather
/// than the generator.
pub const MAX_ARRAY_LEN: usize = 256;

/// The deepest node nesting the format permits.
///
/// Both sides enforce it: the const encoder asserts it while writing, so an
/// encoding that exceeds it fails const evaluation, and the runtime decoder
/// fails with [`DecodeError::TooDeep`](crate::DecodeError::TooDeep) past it —
/// decoding is recursive, so an unbounded payload would recurse unboundedly
/// and overflow the stack. Sixty-four is far past any real props schema while
/// keeping the deepest legal payload inside a small stack budget.
pub const MAX_DEPTH: usize = 64;

/// Node tags. Discriminants are written by hand rather than derived from
/// declaration order so that reordering [`TypeSchema`](crate::TypeSchema)
/// cannot silently change the wire format.
pub(crate) mod tag {
    /// [`TypeSchema::Unit`](crate::TypeSchema::Unit).
    pub const UNIT: u8 = 1;
    /// [`TypeSchema::Bool`](crate::TypeSchema::Bool).
    pub const BOOL: u8 = 2;
    /// [`TypeSchema::Number`](crate::TypeSchema::Number), followed by a kind byte.
    pub const NUMBER: u8 = 3;
    /// [`TypeSchema::String`](crate::TypeSchema::String).
    pub const STRING: u8 = 4;
    /// [`TypeSchema::Option`](crate::TypeSchema::Option), followed by its node.
    pub const OPTION: u8 = 5;
    /// [`TypeSchema::List`](crate::TypeSchema::List), followed by its node.
    pub const LIST: u8 = 6;
    /// [`TypeSchema::Map`](crate::TypeSchema::Map), followed by key then value.
    pub const MAP: u8 = 7;
    /// [`TypeSchema::Signal`](crate::TypeSchema::Signal), followed by its node.
    pub const SIGNAL: u8 = 8;
    /// [`TypeSchema::Accessor`](crate::TypeSchema::Accessor), followed by its node.
    pub const ACCESSOR: u8 = 9;
    /// [`TypeSchema::View`](crate::TypeSchema::View).
    pub const VIEW: u8 = 10;
    /// [`TypeSchema::Callback`](crate::TypeSchema::Callback), followed by a
    /// count and that many argument nodes.
    pub const CALLBACK: u8 = 11;
    /// [`TypeSchema::Struct`](crate::TypeSchema::Struct).
    pub const STRUCT: u8 = 12;
    /// [`TypeSchema::Enum`](crate::TypeSchema::Enum).
    pub const ENUM: u8 = 13;
    /// [`TypeSchema::Array`](crate::TypeSchema::Array), followed by its length
    /// and its element node.
    pub const ARRAY: u8 = 14;
    /// [`TypeSchema::Union`](crate::TypeSchema::Union), followed by a count and
    /// that many member nodes.
    pub const UNION: u8 = 15;
    /// [`TypeSchema::ViewBuilder`](crate::TypeSchema::ViewBuilder).
    pub const VIEW_BUILDER: u8 = 16;
}

/// What a payload carries, written straight after the version byte.
///
/// Four kinds of payload share the format and the version: a props contract,
/// which is one type tree; the component catalog; a mount point; and one half
/// of the runtime fingerprint. A type tree starts with a node tag, so the
/// other three announce themselves with bytes no node tag uses, and each
/// decoder refuses the kinds that are not its own rather than reading one as
/// a malformed payload of its own.
pub(crate) mod kind {
    /// The component catalog: [`decode_catalog`](crate::decode_catalog) reads
    /// it, and [`decode`](crate::decode) refuses it.
    pub const CATALOG: u8 = 0x20;
    /// One `tsx!` mount point: [`decode_mount`](crate::decode_mount) reads it,
    /// and the other decoders refuse it.
    pub const MOUNT: u8 = 0x21;
    /// One half of the runtime fingerprint — the JavaScript library's hash or
    /// the component catalog's: [`decode_runtime_half`](crate::decode_runtime_half)
    /// reads it, and the other decoders refuse it.
    pub const RUNTIME_HALF: u8 = 0x22;
}

/// Which half of the runtime fingerprint a [`kind::RUNTIME_HALF`] payload
/// carries, written straight after the kind byte.
pub(crate) mod runtime_part {
    /// [`RuntimePart::Library`](crate::RuntimePart::Library).
    pub const LIBRARY: u8 = 1;
    /// [`RuntimePart::Catalog`](crate::RuntimePart::Catalog).
    pub const CATALOG: u8 = 2;
}

/// What the byte after the version says a payload is, for the error a decoder
/// handed the wrong kind raises.
pub(crate) const fn payload_kind(byte: u8) -> &'static str {
    match byte {
        kind::CATALOG => "a component catalog",
        kind::MOUNT => "a mount point",
        kind::RUNTIME_HALF => "a runtime fingerprint half",
        _ => "a props type tree",
    }
}

/// Which slot a component's JSX children fill.
pub(crate) mod children {
    /// [`ChildrenSlot::None`](crate::ChildrenSlot::None).
    pub const NONE: u8 = 1;
    /// [`ChildrenSlot::Content`](crate::ChildrenSlot::Content).
    pub const CONTENT: u8 = 2;
    /// [`ChildrenSlot::Label`](crate::ChildrenSlot::Label).
    pub const LABEL: u8 = 3;
    /// [`ChildrenSlot::Text`](crate::ChildrenSlot::Text).
    pub const TEXT: u8 = 4;
}

/// Enum representation tags.
pub(crate) mod representation {
    /// [`EnumRepresentation::StringUnion`](crate::EnumRepresentation::StringUnion).
    pub const STRING_UNION: u8 = 1;
    /// [`EnumRepresentation::Tagged`](crate::EnumRepresentation::Tagged),
    /// followed by the tag and content property names.
    pub const TAGGED: u8 = 2;
}

/// Variant payload tags.
pub(crate) mod variant {
    /// [`VariantPayload::Unit`](crate::VariantPayload::Unit).
    pub const UNIT: u8 = 1;
    /// [`VariantPayload::Tuple`](crate::VariantPayload::Tuple).
    pub const TUPLE: u8 = 2;
    /// [`VariantPayload::Struct`](crate::VariantPayload::Struct).
    pub const STRUCT: u8 = 3;
}
