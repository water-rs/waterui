//! Format-level tests: encoding invariants and decoder rejections.
//!
//! The end-to-end test — derive, then read the payload back out of the
//! compiled artifact's symbol table — lives in `tests/artifact.rs`, because it
//! has to look at the binary it is running from.

use crate::{
    DecodeError, EnumRepresentation, EnumSchema, FieldSchema, NumberKind, StructSchema, TsType,
    TypeSchema, VariantPayload, VariantSchema, contract_hash, decode, encode, encoded_len, owned,
    payload,
};

/// A three-level tree exercising every node kind that carries children.
const NESTED: TypeSchema = TypeSchema::Struct(StructSchema {
    name: "Root",
    fields: &[
        FieldSchema {
            name: "state",
            ty: TypeSchema::Enum(EnumSchema {
                name: "State",
                representation: EnumRepresentation::DEFAULT_TAGGED,
                variants: &[
                    VariantSchema {
                        name: "Idle",
                        payload: VariantPayload::Unit,
                    },
                    VariantSchema {
                        name: "Loading",
                        payload: VariantPayload::Tuple(&[TypeSchema::Number(NumberKind::U32)]),
                    },
                    VariantSchema {
                        name: "Failed",
                        payload: VariantPayload::Struct(&[FieldSchema {
                            name: "message",
                            ty: TypeSchema::String,
                        }]),
                    },
                ],
            }),
        },
        FieldSchema {
            name: "tags",
            ty: TypeSchema::List(&TypeSchema::Option(&TypeSchema::String)),
        },
        FieldSchema {
            name: "counts",
            ty: TypeSchema::Map {
                key: &TypeSchema::String,
                value: &TypeSchema::Number(NumberKind::U64),
            },
        },
        FieldSchema {
            name: "on_pick",
            ty: TypeSchema::Callback(&[TypeSchema::Number(NumberKind::I32), TypeSchema::Bool]),
        },
        FieldSchema {
            name: "slot",
            ty: TypeSchema::View,
        },
    ],
});

/// The encoding of [`NESTED`], terminator included.
const NESTED_ENCODED: [u8; encoded_len(&NESTED) + 1] = encode(&NESTED);

#[test]
fn encoding_is_nul_free_and_nul_terminated() {
    let bytes = NESTED_ENCODED;
    assert_eq!(bytes[bytes.len() - 1], 0, "the encoding is NUL-terminated");
    assert!(
        !payload(&bytes).contains(&0),
        "no payload byte may be NUL: the CLI cuts a static at its first NUL"
    );
}

#[test]
fn decoding_an_encoded_tree_yields_the_same_tree() {
    assert_eq!(
        decode(payload(&NESTED_ENCODED)).expect("the encoding decodes"),
        owned::Schema::from(&NESTED)
    );
}

#[test]
fn decoding_rejects_an_unknown_format_version() {
    let mut bytes = payload(&NESTED_ENCODED).to_vec();
    bytes[0] = crate::FORMAT_VERSION + 1;
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::Version {
            found: crate::FORMAT_VERSION + 1,
            expected: crate::FORMAT_VERSION,
        })
    );
}

#[test]
fn decoding_rejects_a_truncated_payload() {
    let full = payload(&NESTED_ENCODED);
    for length in 1..full.len() {
        assert!(
            decode(&full[..length]).is_err(),
            "a payload cut to {length} of {} bytes must not decode",
            full.len()
        );
    }
    assert_eq!(decode(&[]), Err(DecodeError::Empty));
}

#[test]
fn decoding_rejects_trailing_bytes() {
    let mut bytes = payload(&NESTED_ENCODED).to_vec();
    bytes.push(1);
    assert_eq!(decode(&bytes), Err(DecodeError::Trailing { extra: 1 }));
}

#[test]
fn lengths_above_one_digit_round_trip() {
    // 200 fields forces a two-byte varint, which the small fixtures never do.
    const WIDE: TypeSchema = TypeSchema::Callback(&[TypeSchema::Bool; 200]);
    const ENCODED: [u8; encoded_len(&WIDE) + 1] = encode(&WIDE);
    assert_eq!(
        decode(payload(&ENCODED)).expect("the encoding decodes"),
        owned::Schema::from(&WIDE)
    );
}

#[test]
fn schemas_of_mapped_types_compose() {
    assert_eq!(
        <Vec<Option<String>> as TsType>::SCHEMA,
        TypeSchema::List(&TypeSchema::Option(&TypeSchema::String))
    );
    assert_eq!(
        <Box<dyn Fn(u32, bool)> as TsType>::SCHEMA,
        TypeSchema::Callback(&[TypeSchema::Number(NumberKind::U32), TypeSchema::Bool])
    );
}

#[test]
fn the_contract_hash_covers_the_whole_payload() {
    assert_eq!(
        contract_hash(payload(&NESTED_ENCODED)),
        const_fnv1a_hash::fnv1a_hash_64(payload(&NESTED_ENCODED), None)
    );
}

#[test]
fn display_renders_typescript_type_expressions() {
    assert_eq!(
        TypeSchema::Signal(&TypeSchema::List(&TypeSchema::Number(NumberKind::U64))).to_string(),
        "Signal<bigint[]>"
    );
    assert_eq!(NESTED.to_string(), "Root");
}

/// Hash stability: what the contract hash must and must not notice.
///
/// Every fixture is named `Props` so the comparison isolates one difference at
/// a time. The type name is part of the contract, and two identically shaped
/// `Props` types encode identically — which is the reason the alias fixtures
/// can share a metadata symbol leaf without conflicting.
#[cfg(feature = "derive")]
mod hashing {
    /// Written with concrete types.
    mod spelled {
        use crate::TsProps;

        #[derive(TsProps)]
        #[expect(
            dead_code,
            reason = "the schema is derived from the declaration; nothing constructs the fixture"
        )]
        pub struct Props {
            handle: String,
            counts: Vec<u32>,
        }
    }

    /// The same shape reached through aliases.
    mod aliased {
        use crate::TsProps;

        type Handle = String;
        type Counts = Vec<Tally>;
        type Tally = u32;

        #[derive(TsProps)]
        #[expect(
            dead_code,
            reason = "the schema is derived from the declaration; nothing constructs the fixture"
        )]
        pub struct Props {
            handle: Handle,
            counts: Counts,
        }
    }

    /// The same shape with one field renamed.
    mod renamed {
        use crate::TsProps;

        #[derive(TsProps)]
        #[expect(
            dead_code,
            reason = "the schema is derived from the declaration; nothing constructs the fixture"
        )]
        pub struct Props {
            label: String,
            counts: Vec<u32>,
        }
    }

    /// The same shape with one field made two-way.
    #[cfg(feature = "waterui")]
    mod two_way {
        use crate::TsProps;
        use waterui_core::Binding;

        #[derive(TsProps)]
        #[expect(
            dead_code,
            reason = "the schema is derived from the declaration; nothing constructs the fixture"
        )]
        pub struct Props {
            handle: Binding<String>,
            counts: Vec<u32>,
        }
    }

    /// The same shape with that field read-only instead.
    #[cfg(feature = "waterui")]
    mod read_only {
        use crate::TsProps;
        use waterui_core::Computed;

        #[derive(TsProps)]
        #[expect(
            dead_code,
            reason = "the schema is derived from the declaration; nothing constructs the fixture"
        )]
        pub struct Props {
            handle: Computed<String>,
            counts: Vec<u32>,
        }
    }

    use crate::TsProps as _;

    #[test]
    fn aliases_do_not_change_the_contract() {
        assert_eq!(
            spelled::Props::CONTRACT_HASH,
            aliased::Props::CONTRACT_HASH,
            "the schema records resolved shapes, never how a type is spelled"
        );
        assert_eq!(spelled::Props::ENCODED, aliased::Props::ENCODED);
    }

    #[test]
    fn renaming_a_field_changes_the_contract() {
        assert_ne!(spelled::Props::CONTRACT_HASH, renamed::Props::CONTRACT_HASH);
    }

    #[cfg(feature = "waterui")]
    #[test]
    fn a_two_way_field_is_a_different_contract_than_a_read_only_one() {
        assert_ne!(
            two_way::Props::CONTRACT_HASH,
            read_only::Props::CONTRACT_HASH
        );
        assert_ne!(two_way::Props::CONTRACT_HASH, spelled::Props::CONTRACT_HASH);
    }
}
