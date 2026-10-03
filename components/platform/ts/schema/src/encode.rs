//! The const encoder.
//!
//! One walk serves both purposes: measuring and writing. [`put`] counts every
//! byte and stores it only when the destination has room, so calling the walk
//! with an empty buffer yields the exact length and calling it with a buffer
//! of that length fills it. There is no second traversal to keep in step with
//! the first.

use crate::format::{FORMAT_VERSION, representation, tag, variant};
use crate::tree::{EnumRepresentation, EnumSchema, FieldSchema, TypeSchema, VariantPayload};

/// Number of distinct digits a length varint uses.
///
/// 127 rather than 128: a digit is stored biased by one so that the byte is
/// never zero, and the continuation bit takes the eighth.
const VARINT_RADIX: usize = 127;

/// Append `byte`, storing it when `pos` is inside `buf` and always counting it.
///
/// Passing an empty buffer turns the walk into a measuring pass. The caller is
/// responsible for sizing the real buffer; [`encode`] asserts the size it was
/// handed matches the length this walk reports.
const fn put(buf: &mut [u8], pos: usize, byte: u8) -> usize {
    assert!(
        byte != 0,
        "an encoded schema is NUL-free: the CLI cuts a `#[used]` static at its first NUL"
    );
    if pos < buf.len() {
        buf[pos] = byte;
    }
    pos + 1
}

/// Append `value` as a little-endian base-127 varint.
///
/// Each byte carries one digit biased by one, in `1..=127`, with the high bit
/// set on every byte but the last. No byte can be zero, and no digit sequence
/// has a trailing zero digit, so the encoding is unique.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a remainder of 127 is below u8::MAX by construction"
)]
const fn put_varint(buf: &mut [u8], pos: usize, value: usize) -> usize {
    let mut pos = pos;
    let mut value = value;
    loop {
        let digit = (value % VARINT_RADIX) as u8;
        value /= VARINT_RADIX;
        let more = value != 0;
        pos = put(buf, pos, if more { 0x80 | (digit + 1) } else { digit + 1 });
        if !more {
            return pos;
        }
    }
}

/// Append `text` as a length-prefixed UTF-8 string.
const fn put_str(buf: &mut [u8], pos: usize, text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut pos = put_varint(buf, pos, bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        pos = put(buf, pos, bytes[index]);
        index += 1;
    }
    pos
}

/// Append a sequence of nodes, count first.
const fn put_nodes(buf: &mut [u8], pos: usize, nodes: &[TypeSchema]) -> usize {
    let mut pos = put_varint(buf, pos, nodes.len());
    let mut index = 0;
    while index < nodes.len() {
        pos = put_node(buf, pos, &nodes[index]);
        index += 1;
    }
    pos
}

/// Append a sequence of named fields, count first.
const fn put_fields(buf: &mut [u8], pos: usize, fields: &[FieldSchema]) -> usize {
    let mut pos = put_varint(buf, pos, fields.len());
    let mut index = 0;
    while index < fields.len() {
        pos = put_str(buf, pos, fields[index].name);
        pos = put_node(buf, pos, &fields[index].ty);
        index += 1;
    }
    pos
}

/// Append an enum body: name, representation, then the variants.
const fn put_enum(buf: &mut [u8], pos: usize, schema: &EnumSchema) -> usize {
    let mut pos = put_str(buf, pos, schema.name);
    pos = match schema.representation {
        EnumRepresentation::StringUnion => put(buf, pos, representation::STRING_UNION),
        EnumRepresentation::Tagged {
            tag: tag_property,
            content: content_property,
        } => {
            let pos = put(buf, pos, representation::TAGGED);
            let pos = put_str(buf, pos, tag_property);
            put_str(buf, pos, content_property)
        }
    };
    pos = put_varint(buf, pos, schema.variants.len());
    let mut index = 0;
    while index < schema.variants.len() {
        let case = &schema.variants[index];
        pos = put_str(buf, pos, case.name);
        pos = match case.payload {
            VariantPayload::Unit => put(buf, pos, variant::UNIT),
            VariantPayload::Tuple(nodes) => {
                let pos = put(buf, pos, variant::TUPLE);
                put_nodes(buf, pos, nodes)
            }
            VariantPayload::Struct(fields) => {
                let pos = put(buf, pos, variant::STRUCT);
                put_fields(buf, pos, fields)
            }
        };
        index += 1;
    }
    pos
}

/// Append one node and everything below it.
const fn put_node(buf: &mut [u8], pos: usize, node: &TypeSchema) -> usize {
    match node {
        TypeSchema::Unit => put(buf, pos, tag::UNIT),
        TypeSchema::Bool => put(buf, pos, tag::BOOL),
        TypeSchema::Number(kind) => {
            let pos = put(buf, pos, tag::NUMBER);
            put(buf, pos, kind.tag())
        }
        TypeSchema::String => put(buf, pos, tag::STRING),
        TypeSchema::Option(inner) => {
            let pos = put(buf, pos, tag::OPTION);
            put_node(buf, pos, inner)
        }
        TypeSchema::List(inner) => {
            let pos = put(buf, pos, tag::LIST);
            put_node(buf, pos, inner)
        }
        TypeSchema::Map { key, value } => {
            let pos = put(buf, pos, tag::MAP);
            let pos = put_node(buf, pos, key);
            put_node(buf, pos, value)
        }
        TypeSchema::Signal(inner) => {
            let pos = put(buf, pos, tag::SIGNAL);
            put_node(buf, pos, inner)
        }
        TypeSchema::Accessor(inner) => {
            let pos = put(buf, pos, tag::ACCESSOR);
            put_node(buf, pos, inner)
        }
        TypeSchema::View => put(buf, pos, tag::VIEW),
        TypeSchema::Callback(arguments) => {
            let pos = put(buf, pos, tag::CALLBACK);
            put_nodes(buf, pos, arguments)
        }
        TypeSchema::Struct(schema) => {
            let pos = put(buf, pos, tag::STRUCT);
            let pos = put_str(buf, pos, schema.name);
            put_fields(buf, pos, schema.fields)
        }
        TypeSchema::Enum(schema) => {
            let pos = put(buf, pos, tag::ENUM);
            put_enum(buf, pos, schema)
        }
    }
}

/// Write the version byte and the whole tree into `buf`, returning the length.
const fn put_payload(buf: &mut [u8], schema: &TypeSchema) -> usize {
    let pos = put(buf, 0, FORMAT_VERSION);
    put_node(buf, pos, schema)
}

/// Length in bytes of `schema`'s encoded payload, not counting the NUL
/// terminator the artifact static appends.
#[must_use]
pub const fn encoded_len(schema: &TypeSchema) -> usize {
    let mut probe: [u8; 0] = [];
    put_payload(&mut probe, schema)
}

/// Encode `schema` into a NUL-terminated array.
///
/// `N` must be `encoded_len(schema) + 1`: the payload plus the single
/// terminating NUL that lets the CLI find the payload's end in a section whose
/// symbols carry no size. A mismatched `N` fails const evaluation, so the
/// derive that computes both from the same constant cannot drift.
///
/// # Panics
/// Fails const evaluation when `N` is not `encoded_len(schema) + 1`.
#[must_use]
pub const fn encode<const N: usize>(schema: &TypeSchema) -> [u8; N] {
    let mut encoded = [0_u8; N];
    let end = put_payload(&mut encoded, schema);
    assert!(
        end + 1 == N,
        "encode::<N> requires N == encoded_len(schema) + 1"
    );
    encoded
}

/// The payload inside a NUL-terminated encoding: everything but the
/// terminator, which is the byte range the CLI reads back from an artifact.
///
/// # Panics
/// Fails const evaluation when `encoded` is empty or is not NUL-terminated.
#[must_use]
pub const fn payload(encoded: &[u8]) -> &[u8] {
    assert!(!encoded.is_empty(), "an encoding is never empty");
    let (payload, terminator) = encoded.split_at(encoded.len() - 1);
    assert!(terminator[0] == 0, "an encoding is NUL-terminated");
    payload
}

/// The 64-bit FNV-1a hash of an encoded payload: the props contract hash.
///
/// Two structs of the same shape hash identically however their field types
/// are spelled, because the encoding records resolved shapes and never source
/// text. Renaming a field, reordering fields, or turning a `Binding` into a
/// `Computed` all change the bytes and therefore the hash.
#[must_use]
pub const fn contract_hash(payload: &[u8]) -> u64 {
    const_fnv1a_hash::fnv1a_hash_64(payload, None)
}
