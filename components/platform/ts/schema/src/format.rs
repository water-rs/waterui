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
pub const FORMAT_VERSION: u8 = 1;

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
