//! The runtime decoder.
//!
//! Reads an encoded payload — the bytes the `water` CLI recovers from a
//! `waterui_meta_tsprops_*` static — back into an [`owned::Schema`]. Every
//! malformed input is an error, never a partial or guessed tree.

use crate::format::{FORMAT_VERSION, MAX_ARRAY_LEN, MAX_DEPTH, representation, tag, variant};
use crate::owned;
use crate::tree::NumberKind;

/// Why an encoded schema payload could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The payload is empty, so it does not even carry a version byte.
    #[error("the schema payload is empty")]
    Empty,
    /// The payload was produced by a different format version.
    #[error("schema payload format version {found} is not the version {expected} this build reads")]
    Version {
        /// The version byte the payload carries.
        found: u8,
        /// The version this build implements.
        expected: u8,
    },
    /// The payload ended while a value was still expected.
    #[error("the schema payload ends at byte {offset} while more input was required")]
    Truncated {
        /// Where the read ran off the end.
        offset: usize,
    },
    /// A tag byte does not name anything this version defines.
    #[error("byte {offset} holds {tag}, which is not a known {kind} tag")]
    UnknownTag {
        /// What was being read: `node`, `number`, `length`, `representation`
        /// or `variant`.
        kind: &'static str,
        /// The byte that was read.
        tag: u8,
        /// Where it was read.
        offset: usize,
    },
    /// A fixed-length array is longer than the projection allows.
    #[error(
        "the array at byte {offset} carries {len} elements, more than the {limit} a tuple type \
         projects"
    )]
    ArrayTooLong {
        /// The length the payload carries.
        len: usize,
        /// The largest length the format projects.
        limit: usize,
        /// Where the length starts.
        offset: usize,
    },

    /// A length does not fit in a `usize` on this platform.
    #[error("the length at byte {offset} does not fit in a usize")]
    LengthOverflow {
        /// Where the length starts.
        offset: usize,
    },
    /// A string is not valid UTF-8.
    #[error("the string at byte {offset} is not valid UTF-8")]
    InvalidUtf8 {
        /// Where the string's bytes start.
        offset: usize,
    },
    /// Bytes remain after the root type has been read.
    #[error("the schema payload has {extra} trailing bytes after the root type")]
    Trailing {
        /// How many bytes are left over.
        extra: usize,
    },
    /// The tree nests deeper than [`MAX_DEPTH`]. Decoding is recursive, so a
    /// payload of N nested `Option`/`List`-style tags would otherwise recurse
    /// N frames deep and overflow the stack; the encoder asserts the same
    /// bound during const evaluation.
    #[error("the schema payload nests deeper than {limit} nodes, the most the format allows")]
    TooDeep {
        /// The nesting limit that was exceeded.
        limit: usize,
    },
    /// A `StringUnion` enum's variant carries a payload. Only an enum whose
    /// variants are all unit encodes that way; the encoder asserts the same
    /// invariant during const evaluation.
    #[error(
        "variant `{variant}` of `{enum_name}` carries a payload although the enum is a string union"
    )]
    StringUnionVariant {
        /// The enum the variant belongs to.
        enum_name: String,
        /// The variant carrying the payload.
        variant: String,
        /// Where the variant's payload tag was read.
        offset: usize,
    },
    /// A map's key schema is not a string. `TsMapKey` admits only string
    /// types, and the encoder asserts the key is a string schema, so a valid
    /// payload cannot contain this.
    #[error("the map key node at byte {offset} does not decode to a string schema")]
    NonStringMapKey {
        /// Where the key node starts.
        offset: usize,
    },
    /// A name in the schema is empty. The encoder asserts every name it
    /// writes is non-empty, so a valid payload cannot contain one.
    #[error("the {kind} name at byte {offset} is empty")]
    EmptyName {
        /// What was being named: `struct`, `enum`, `field`, `variant`, `tag`
        /// or `content`.
        kind: &'static str,
        /// Where the empty name's length prefix was read.
        offset: usize,
    },
}

/// Decode a payload into an owned schema tree.
///
/// The payload is the byte range between the version byte and the NUL
/// terminator; [`crate::payload`] produces it from an encoding, and the CLI's
/// artifact reader produces it by cutting a static's section data at its first
/// NUL. A trailing terminator is not accepted — it is not part of the payload.
///
/// # Errors
/// Returns a [`DecodeError`] for an unknown version, a truncated or malformed
/// payload, nesting deeper than [`MAX_DEPTH`], a state that violates an
/// invariant the encoder asserts, or bytes left over after the root type.
pub fn decode(payload: &[u8]) -> Result<owned::Schema, DecodeError> {
    if payload.is_empty() {
        return Err(DecodeError::Empty);
    }
    let mut reader = Reader {
        bytes: payload,
        pos: 0,
        depth: 0,
    };
    let version = reader.byte()?;
    if version != FORMAT_VERSION {
        return Err(DecodeError::Version {
            found: version,
            expected: FORMAT_VERSION,
        });
    }
    let schema = reader.node()?;
    let extra = payload.len() - reader.pos;
    if extra > 0 {
        return Err(DecodeError::Trailing { extra });
    }
    Ok(schema)
}

/// A cursor over an encoded payload.
struct Reader<'a> {
    /// The payload.
    bytes: &'a [u8],
    /// How far the cursor has advanced.
    pos: usize,
    /// How many `node` frames are live, bounded by [`MAX_DEPTH`].
    depth: usize,
}

impl Reader<'_> {
    /// Read one byte.
    fn byte(&mut self) -> Result<u8, DecodeError> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or(DecodeError::Truncated { offset: self.pos })?;
        self.pos += 1;
        Ok(byte)
    }

    /// Read a little-endian base-127 varint.
    fn length(&mut self) -> Result<usize, DecodeError> {
        let offset = self.pos;
        let mut value = 0_usize;
        let mut scale = 1_usize;
        loop {
            let byte = self.byte()?;
            // Digits are stored biased by one so no encoded byte is ever NUL;
            // a zero low nibble group is therefore not a digit at all.
            let biased = byte & 0x7f;
            if biased == 0 {
                return Err(DecodeError::UnknownTag {
                    kind: "length",
                    tag: byte,
                    offset: self.pos - 1,
                });
            }
            let digit = usize::from(biased - 1);
            value = digit
                .checked_mul(scale)
                .and_then(|term| value.checked_add(term))
                .ok_or(DecodeError::LengthOverflow { offset })?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            scale = scale
                .checked_mul(127)
                .ok_or(DecodeError::LengthOverflow { offset })?;
        }
    }

    /// Read a length-prefixed UTF-8 string.
    fn string(&mut self) -> Result<String, DecodeError> {
        let len = self.length()?;
        let offset = self.pos;
        let end = offset
            .checked_add(len)
            .ok_or(DecodeError::LengthOverflow { offset })?;
        let bytes = self
            .bytes
            .get(offset..end)
            .ok_or(DecodeError::Truncated { offset: end })?;
        self.pos = end;
        core::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8 { offset })
    }

    /// Read a counted sequence of nodes.
    fn nodes(&mut self) -> Result<Vec<owned::Schema>, DecodeError> {
        let count = self.length()?;
        let mut nodes = Vec::new();
        for _ in 0..count {
            nodes.push(self.node()?);
        }
        Ok(nodes)
    }

    /// Read a counted sequence of named fields.
    fn fields(&mut self) -> Result<Vec<owned::Field>, DecodeError> {
        let count = self.length()?;
        let mut fields = Vec::new();
        for _ in 0..count {
            let name_offset = self.pos;
            let name = self.string()?;
            if name.is_empty() {
                return Err(DecodeError::EmptyName {
                    kind: "field",
                    offset: name_offset,
                });
            }
            fields.push(owned::Field {
                name,
                ty: self.node()?,
            });
        }
        Ok(fields)
    }

    /// Read a boxed child node.
    fn child(&mut self) -> Result<Box<owned::Schema>, DecodeError> {
        self.node().map(Box::new)
    }

    /// Read one node and everything below it, bounding recursion.
    fn node(&mut self) -> Result<owned::Schema, DecodeError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(DecodeError::TooDeep { limit: MAX_DEPTH });
        }
        // An error return is terminal — `decode` propagates the first one —
        // so the depth bookkeeping only needs to be balanced on success.
        let node = self.node_inner();
        if node.is_ok() {
            self.depth -= 1;
        }
        node
    }

    /// Read one node and everything below it.
    fn node_inner(&mut self) -> Result<owned::Schema, DecodeError> {
        let offset = self.pos;
        let node = self.byte()?;
        Ok(match node {
            tag::UNIT => owned::Schema::Unit,
            tag::BOOL => owned::Schema::Bool,
            tag::NUMBER => {
                let offset = self.pos;
                let kind = self.byte()?;
                owned::Schema::Number(NumberKind::from_tag(kind).ok_or(
                    DecodeError::UnknownTag {
                        kind: "number",
                        tag: kind,
                        offset,
                    },
                )?)
            }
            tag::STRING => owned::Schema::String,
            tag::OPTION => owned::Schema::Option(self.child()?),
            tag::LIST => owned::Schema::List(self.child()?),
            tag::ARRAY => {
                let offset = self.pos;
                let len = self.length()?;
                if len > MAX_ARRAY_LEN {
                    return Err(DecodeError::ArrayTooLong {
                        len,
                        limit: MAX_ARRAY_LEN,
                        offset,
                    });
                }
                owned::Schema::Array {
                    item: self.child()?,
                    len,
                }
            }
            tag::MAP => {
                let key_offset = self.pos;
                let key = self.child()?;
                if !matches!(*key, owned::Schema::String) {
                    return Err(DecodeError::NonStringMapKey { offset: key_offset });
                }
                owned::Schema::Map {
                    key,
                    value: self.child()?,
                }
            }
            tag::SIGNAL => owned::Schema::Signal(self.child()?),
            tag::ACCESSOR => owned::Schema::Accessor(self.child()?),
            tag::VIEW => owned::Schema::View,
            tag::CALLBACK => owned::Schema::Callback(self.nodes()?),
            tag::STRUCT => {
                let name_offset = self.pos;
                let name = self.string()?;
                if name.is_empty() {
                    return Err(DecodeError::EmptyName {
                        kind: "struct",
                        offset: name_offset,
                    });
                }
                owned::Schema::Struct(owned::Struct {
                    name,
                    fields: self.fields()?,
                })
            }
            tag::ENUM => owned::Schema::Enum(self.enumeration()?),
            other => {
                return Err(DecodeError::UnknownTag {
                    kind: "node",
                    tag: other,
                    offset,
                });
            }
        })
    }

    /// Read an enum body.
    fn enumeration(&mut self) -> Result<owned::Enum, DecodeError> {
        let name_offset = self.pos;
        let name = self.string()?;
        if name.is_empty() {
            return Err(DecodeError::EmptyName {
                kind: "enum",
                offset: name_offset,
            });
        }
        let offset = self.pos;
        let representation = match self.byte()? {
            representation::STRING_UNION => owned::Representation::StringUnion,
            representation::TAGGED => {
                let tag_offset = self.pos;
                let tag = self.string()?;
                if tag.is_empty() {
                    return Err(DecodeError::EmptyName {
                        kind: "tag",
                        offset: tag_offset,
                    });
                }
                let content_offset = self.pos;
                let content = self.string()?;
                if content.is_empty() {
                    return Err(DecodeError::EmptyName {
                        kind: "content",
                        offset: content_offset,
                    });
                }
                owned::Representation::Tagged { tag, content }
            }
            other => {
                return Err(DecodeError::UnknownTag {
                    kind: "representation",
                    tag: other,
                    offset,
                });
            }
        };
        let count = self.length()?;
        let mut variants = Vec::new();
        for _ in 0..count {
            let variant_offset = self.pos;
            let variant_name = self.string()?;
            if variant_name.is_empty() {
                return Err(DecodeError::EmptyName {
                    kind: "variant",
                    offset: variant_offset,
                });
            }
            let offset = self.pos;
            let payload_tag = self.byte()?;
            // A string union is only ever written for an all-unit enum; the
            // encoder asserts the same invariant during const evaluation.
            if matches!(representation, owned::Representation::StringUnion)
                && payload_tag != variant::UNIT
            {
                return Err(DecodeError::StringUnionVariant {
                    enum_name: name,
                    variant: variant_name,
                    offset,
                });
            }
            let payload = match payload_tag {
                variant::UNIT => owned::Payload::Unit,
                variant::TUPLE => owned::Payload::Tuple(self.nodes()?),
                variant::STRUCT => owned::Payload::Struct(self.fields()?),
                other => {
                    return Err(DecodeError::UnknownTag {
                        kind: "variant",
                        tag: other,
                        offset,
                    });
                }
            };
            variants.push(owned::Variant {
                name: variant_name,
                payload,
            });
        }
        Ok(owned::Enum {
            name,
            representation,
            variants,
        })
    }
}
