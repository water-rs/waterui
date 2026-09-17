//! Format-level tests: encoding invariants and decoder rejections.
//!
//! The end-to-end test — derive, then read the payload back out of the
//! compiled artifact's symbol table — lives in `tests/artifact.rs`, because it
//! has to look at the binary it is running from.

use std::rc::Rc;
use std::sync::Arc;

use crate::format::{representation, tag, variant};
use crate::{
    DecodeError, EnumRepresentation, EnumSchema, FieldSchema, MAX_DEPTH, NumberKind, StructSchema,
    TsType, TypeSchema, VariantPayload, VariantSchema, contract_hash, decode, encode, encoded_len,
    owned, payload,
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
fn a_fixed_length_array_declares_its_length() {
    const FOUR: TypeSchema = <[u32; 4] as TsType>::SCHEMA;
    const ENCODED: [u8; encoded_len(&FOUR) + 1] = encode(&FOUR);
    const LIST: TypeSchema = <Vec<u32> as TsType>::SCHEMA;
    const LIST_ENCODED: [u8; encoded_len(&LIST) + 1] = encode(&LIST);

    assert_eq!(
        FOUR,
        TypeSchema::Array {
            item: &TypeSchema::Number(NumberKind::U32),
            len: 4,
        },
        "an array is not a list: the conversion refuses any other length"
    );
    assert_eq!(
        FOUR.to_string(),
        "[number, number, number, number]",
        "and the projection says so"
    );
    assert_eq!(
        <[u8; 0] as TsType>::SCHEMA.to_string(),
        "[]",
        "an empty array projects as the empty tuple"
    );

    assert_eq!(
        decode(payload(&ENCODED)).expect("the encoding decodes"),
        owned::Schema::from(&FOUR)
    );
    assert_ne!(
        contract_hash(payload(&ENCODED)),
        contract_hash(payload(&LIST_ENCODED)),
        "a fixed-length array and a list are different contracts"
    );
}

#[test]
fn every_callable_spelling_is_a_callback() {
    const ONE_NUMBER: TypeSchema = TypeSchema::Callback(&[TypeSchema::Number(NumberKind::U32)]);
    assert_eq!(<Box<dyn Fn(u32)> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(<Box<dyn Fn(u32) + Send> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(<Box<dyn Fn(u32) + Sync> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(
        <Box<dyn Fn(u32) + Send + Sync> as TsType>::SCHEMA,
        ONE_NUMBER
    );
    assert_eq!(<Arc<dyn Fn(u32)> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(<Arc<dyn Fn(u32) + Send> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(<Arc<dyn Fn(u32) + Sync> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(
        <Arc<dyn Fn(u32) + Send + Sync> as TsType>::SCHEMA,
        ONE_NUMBER
    );
    assert_eq!(<Rc<dyn Fn(u32)> as TsType>::SCHEMA, ONE_NUMBER);
    assert_eq!(<fn(u32) as TsType>::SCHEMA, ONE_NUMBER);

    // The documented cap is eight arguments.
    assert!(matches!(
        <fn(u32, u32, u32, u32, u32, u32, u32, u32) as TsType>::SCHEMA,
        TypeSchema::Callback(arguments) if arguments.len() == 8
    ));
}

#[test]
fn decoding_rejects_nesting_beyond_the_depth_limit() {
    // `MAX_DEPTH + 1` nested `Option` tags would recurse that many frames
    // without a bound; the decoder must fail rather than overflow the stack.
    let mut bytes = Vec::with_capacity(MAX_DEPTH + 3);
    bytes.push(crate::FORMAT_VERSION);
    bytes.extend(std::iter::repeat_n(tag::OPTION, MAX_DEPTH + 1));
    bytes.push(tag::UNIT);
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::TooDeep { limit: MAX_DEPTH })
    );

    // One level shallower the leaf still sits inside the limit and decodes:
    // `MAX_DEPTH - 1` options plus the leaf is exactly `MAX_DEPTH` deep.
    let mut shallow = Vec::with_capacity(MAX_DEPTH + 2);
    shallow.push(crate::FORMAT_VERSION);
    shallow.extend(std::iter::repeat_n(tag::OPTION, MAX_DEPTH - 1));
    shallow.push(tag::UNIT);
    assert!(matches!(decode(&shallow), Ok(owned::Schema::Option(_))));
}

#[test]
fn decoding_rejects_a_string_union_variant_with_a_payload() {
    // A `StringUnion` enum's variants are all unit by definition, so a tuple
    // payload violates an invariant the encoder also asserts.
    let bytes = [
        crate::FORMAT_VERSION,
        tag::ENUM,
        2,
        b'E',
        representation::STRING_UNION,
        2, // one variant
        2,
        b'V',
        variant::TUPLE,
        2, // one node
        tag::BOOL,
    ];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::StringUnionVariant {
            enum_name: "E".into(),
            variant: "V".into(),
            offset: 8,
        })
    );
}

#[test]
fn decoding_rejects_a_map_with_a_non_string_key() {
    // `TsMapKey` admits only string types, so a non-string key violates an
    // invariant the encoder also asserts.
    let bytes = [
        crate::FORMAT_VERSION,
        tag::MAP,
        tag::BOOL,   // key
        tag::STRING, // value
    ];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::NonStringMapKey { offset: 2 })
    );
}

#[test]
fn decoding_rejects_a_tagged_enum_with_an_empty_property_name() {
    // A length-0 string encodes as the single byte `1`; the encoder asserts
    // every name it writes is non-empty, so an empty one cannot appear in a
    // valid payload.
    let bytes = [
        crate::FORMAT_VERSION,
        tag::ENUM,
        2,
        b'E',
        representation::TAGGED,
        1, // empty tag name
        2,
        b'v',
    ];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::EmptyName {
            kind: "tag",
            offset: 5,
        })
    );

    let bytes = [
        crate::FORMAT_VERSION,
        tag::ENUM,
        2,
        b'E',
        representation::TAGGED,
        2,
        b't',
        1, // empty content name
    ];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::EmptyName {
            kind: "content",
            offset: 7,
        })
    );
}

#[test]
fn decoding_rejects_empty_names() {
    // The same empty-name invariant, at each position a name can occupy.
    let bytes = [crate::FORMAT_VERSION, tag::STRUCT, 1 /* empty name */];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::EmptyName {
            kind: "struct",
            offset: 2,
        })
    );

    let bytes = [crate::FORMAT_VERSION, tag::ENUM, 1 /* empty name */];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::EmptyName {
            kind: "enum",
            offset: 2,
        })
    );

    let bytes = [
        crate::FORMAT_VERSION,
        tag::STRUCT,
        2,
        b'S',
        2, // one field
        1, // empty field name
    ];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::EmptyName {
            kind: "field",
            offset: 5,
        })
    );

    let bytes = [
        crate::FORMAT_VERSION,
        tag::ENUM,
        2,
        b'E',
        representation::STRING_UNION,
        2, // one variant
        1, // empty variant name
    ];
    assert_eq!(
        decode(&bytes),
        Err(DecodeError::EmptyName {
            kind: "variant",
            offset: 6,
        })
    );
}

/// The encoder asserts the same invariants the decoder checks, so a
/// hand-built `TypeSchema` that violates one fails const evaluation rather
/// than producing an encoding nothing can read back.
mod encoder_invariants {
    use crate::{EnumRepresentation, EnumSchema, TypeSchema, VariantPayload, VariantSchema};
    use crate::{StructSchema, encoded_len};

    #[test]
    #[should_panic(expected = "cannot be projected as a TypeScript tuple type")]
    fn encoding_rejects_an_array_longer_than_the_projection() {
        const BAD: TypeSchema = TypeSchema::Array {
            item: &TypeSchema::Bool,
            len: crate::MAX_ARRAY_LEN + 1,
        };
        let _ = encoded_len(&BAD);
    }

    #[test]
    #[should_panic(expected = "a map crossing the props seam has a string key")]
    fn encoding_rejects_a_map_with_a_non_string_key() {
        const BAD: TypeSchema = TypeSchema::Map {
            key: &TypeSchema::Bool,
            value: &TypeSchema::String,
        };
        let _ = encoded_len(&BAD);
    }

    #[test]
    #[should_panic(expected = "a string-union variant cannot carry a payload")]
    fn encoding_rejects_a_string_union_variant_with_a_payload() {
        const BAD: TypeSchema = TypeSchema::Enum(EnumSchema {
            name: "E",
            representation: EnumRepresentation::StringUnion,
            variants: &[VariantSchema {
                name: "V",
                payload: VariantPayload::Tuple(&[TypeSchema::Bool]),
            }],
        });
        let _ = encoded_len(&BAD);
    }

    #[test]
    #[should_panic(expected = "a struct name must not be empty")]
    fn encoding_rejects_an_empty_struct_name() {
        const BAD: TypeSchema = TypeSchema::Struct(StructSchema {
            name: "",
            fields: &[],
        });
        let _ = encoded_len(&BAD);
    }

    #[test]
    #[should_panic(expected = "a tagged enum's `tag` property name must not be empty")]
    fn encoding_rejects_an_empty_tagged_property_name() {
        const BAD: TypeSchema = TypeSchema::Enum(EnumSchema {
            name: "E",
            representation: EnumRepresentation::Tagged {
                tag: "",
                content: "content",
            },
            variants: &[],
        });
        let _ = encoded_len(&BAD);
    }
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

/// Raw identifiers: `r#` is a spelling detail, not part of the name, so it
/// must not leak into the projected schema or the artifact symbol.
#[cfg(feature = "derive")]
mod raw_identifiers {
    use crate::{TsProps, TsType, TypeSchema};

    #[derive(TsType)]
    #[expect(
        dead_code,
        reason = "the schema is derived from the declaration; nothing constructs the fixture"
    )]
    struct Kinded {
        r#type: String,
    }

    #[derive(TsType)]
    #[expect(
        dead_code,
        reason = "the schema is derived from the declaration; nothing constructs the fixture"
    )]
    #[expect(
        non_camel_case_types,
        reason = "raw keyword variants are deliberately spelled like the keywords they test"
    )]
    enum Keywords {
        r#match,
        r#type,
    }

    #[derive(TsProps)]
    #[expect(
        dead_code,
        reason = "the schema is derived from the declaration; nothing constructs the fixture"
    )]
    struct r#Type {
        handle: String,
    }

    #[test]
    fn a_raw_field_name_projects_without_the_raw_prefix() {
        let TypeSchema::Struct(schema) = Kinded::SCHEMA else {
            panic!("`Kinded` is a struct")
        };
        assert_eq!(schema.fields[0].name, "type");
    }

    #[test]
    fn raw_variant_names_project_without_the_raw_prefix() {
        let TypeSchema::Enum(schema) = Keywords::SCHEMA else {
            panic!("`Keywords` is an enum")
        };
        assert_eq!(schema.variants[0].name, "match");
        assert_eq!(schema.variants[1].name, "type");
    }

    #[test]
    fn a_raw_type_name_gives_an_unrawed_meta_static() {
        let TypeSchema::Struct(schema) = r#Type::SCHEMA else {
            panic!("`r#Type` is a struct")
        };
        assert_eq!(schema.name, "Type");
        // The metadata static exists only in debug builds.
        #[cfg(debug_assertions)]
        assert_eq!(crate::payload(&waterui_meta_tsprops_Type), r#Type::ENCODED);
    }
}

/// The component catalog: the second payload kind the format carries.
mod catalog {
    use crate::{
        Catalog, CatalogSchema, ChildrenSlot, ComponentSchema, FieldSchema, ModifierSchema,
        NumberKind, StructSchema, TypeSchema, attributes_of, catalog_encoded_len, decode,
        decode_catalog, encode_catalog, payload,
    };

    /// `true | number | { top: number }` — the shape a union exists for.
    const PADDING: TypeSchema = TypeSchema::Union(&[
        TypeSchema::Bool,
        TypeSchema::Number(NumberKind::F64),
        TypeSchema::Struct(StructSchema {
            name: "EdgeInsets",
            fields: &[FieldSchema {
                name: "top",
                ty: TypeSchema::Number(NumberKind::F64),
            }],
        }),
    ]);

    const TOGGLE_ATTRIBUTES: TypeSchema = TypeSchema::Struct(StructSchema {
        name: "ToggleAttributes",
        fields: &[FieldSchema {
            name: "value",
            ty: TypeSchema::Signal(&TypeSchema::Bool),
        }],
    });

    const SPACER_ATTRIBUTES: TypeSchema = TypeSchema::Struct(StructSchema {
        name: "SpacerAttributes",
        fields: &[],
    });

    const CATALOG: CatalogSchema = CatalogSchema {
        components: &[
            ComponentSchema {
                name: "Toggle",
                summary: "A two-state switch.",
                attributes: attributes_of(&TOGGLE_ATTRIBUTES),
                children: ChildrenSlot::Label,
            },
            ComponentSchema {
                name: "Spacer",
                summary: "A flexible gap.",
                attributes: attributes_of(&SPACER_ATTRIBUTES),
                children: ChildrenSlot::None,
            },
        ],
        modifiers: &[ModifierSchema {
            name: "padding",
            summary: "Insets the view.",
            value: &PADDING,
        }],
    };

    const ENCODED: [u8; catalog_encoded_len(&CATALOG) + 1] = encode_catalog(&CATALOG);

    #[test]
    fn a_catalog_decodes_to_the_constant_the_compiler_encoded() {
        let decoded = decode_catalog(payload(&ENCODED)).expect("the catalog payload decodes");
        assert_eq!(decoded, Catalog::from(&CATALOG));
        assert_eq!(decoded.components[0].attributes[0].name, "value");
        assert_eq!(decoded.components[1].attributes, []);
        assert_eq!(
            decoded.modifiers[0].value,
            crate::owned::Schema::from(&PADDING)
        );
    }

    #[test]
    fn the_catalog_payload_is_nul_free_so_the_cli_can_find_its_end() {
        assert!(!payload(&ENCODED).contains(&0));
    }

    #[test]
    fn the_modifier_order_sentence_is_a_sentence() {
        // The generator copies the constant into every modifier's doc
        // comment verbatim, so it has to be a complete sentence here —
        // a fragment or a missing full stop would be copied everywhere.
        assert!(
            !crate::MODIFIER_ORDER_SENTENCE.is_empty(),
            "the sentence every modifier's doc opens with cannot be empty"
        );
        assert!(
            crate::MODIFIER_ORDER_SENTENCE.ends_with('.'),
            "the sentence must end with a full stop: {:?}",
            crate::MODIFIER_ORDER_SENTENCE
        );
    }

    #[test]
    fn a_union_renders_as_the_typescript_union() {
        assert_eq!(PADDING.to_string(), "boolean | number | EdgeInsets");
    }

    #[test]
    fn a_union_inside_a_list_is_parenthesised() {
        // `T[]` binds tighter than `|`, so without the parentheses this reads
        // as "a boolean, or an array of numbers" — a different type.
        const MEMBERS: [TypeSchema; 2] = [TypeSchema::Bool, TypeSchema::Number(NumberKind::F32)];
        const UNION: TypeSchema = TypeSchema::Union(&MEMBERS);
        const LIST: TypeSchema = TypeSchema::List(&UNION);
        assert_eq!(LIST.to_string(), "(boolean | number)[]");
    }

    #[test]
    fn an_option_inside_a_list_is_parenthesised() {
        const ITEM: TypeSchema = TypeSchema::String;
        const OPTION: TypeSchema = TypeSchema::Option(&ITEM);
        const LIST: TypeSchema = TypeSchema::List(&OPTION);
        assert_eq!(LIST.to_string(), "(string | null)[]");
    }

    #[test]
    fn a_plain_element_type_keeps_its_bare_suffix() {
        const LIST: TypeSchema = TypeSchema::List(&TypeSchema::String);
        assert_eq!(LIST.to_string(), "string[]");
    }

    #[test]
    fn a_view_builder_round_trips_and_renders_as_a_thunk() {
        const BUILDER: TypeSchema = TypeSchema::ViewBuilder;
        const ENCODED_BUILDER: [u8; crate::encoded_len(&BUILDER) + 1] =
            crate::encode::<{ crate::encoded_len(&BUILDER) + 1 }>(&BUILDER);
        assert_eq!(BUILDER.to_string(), "() => JSX.Element");
        assert_eq!(
            decode(payload(&ENCODED_BUILDER)).expect("a view builder decodes"),
            crate::owned::Schema::ViewBuilder
        );
    }

    #[test]
    fn each_decoder_refuses_the_other_payload_kinds() {
        const MOUNT: [u8; crate::mount_encoded_len("src/promo.tsx", "PromoProps", 7) + 1] =
            crate::encode_mount("src/promo.tsx", "PromoProps", 7);
        let props =
            crate::encode::<{ crate::encoded_len(&TOGGLE_ATTRIBUTES) + 1 }>(&TOGGLE_ATTRIBUTES);
        assert_eq!(
            decode(payload(&ENCODED)).expect_err("a catalog is not a type tree"),
            crate::DecodeError::NotATypeTree {
                found: "a component catalog"
            }
        );
        assert_eq!(
            decode(payload(&MOUNT)).expect_err("a mount point is not a type tree"),
            crate::DecodeError::NotATypeTree {
                found: "a mount point"
            }
        );
        assert_eq!(
            decode_catalog(payload(&props)).expect_err("a props contract is not a catalog"),
            crate::DecodeError::NotACatalog {
                found: "a props type tree"
            }
        );
        assert_eq!(
            crate::decode_mount(payload(&ENCODED)).expect_err("a catalog is not a mount point"),
            crate::DecodeError::NotAMountPoint {
                found: "a component catalog"
            }
        );
    }
}

/// The mount-point payload: one `tsx!` call site, recovered from its bytes.
mod mount {
    use crate::{
        FieldSchema, MountPoint, StructSchema, TypeSchema, decode_mount, encode_mount,
        mount_encoded_len, payload, struct_name,
    };

    /// A props contract whose name the mount point records.
    const PROMO: TypeSchema = TypeSchema::Struct(StructSchema {
        name: "PromoProps",
        fields: &[FieldSchema {
            name: "headline",
            ty: TypeSchema::String,
        }],
    });

    /// A hash with bits set above `u32`, so a decoder reading it at the wrong
    /// width loses something a test can see.
    const HASH: u64 = 0xfedc_ba98_7654_3210;

    const MODULE: &str = "src/views/promo.tsx";

    const ENCODED: [u8; mount_encoded_len(MODULE, struct_name(&PROMO), HASH) + 1] =
        encode_mount(MODULE, struct_name(&PROMO), HASH);

    #[test]
    fn a_mount_point_decodes_to_the_constant_the_compiler_encoded() {
        assert_eq!(
            decode_mount(payload(&ENCODED)).expect("the mount payload decodes"),
            MountPoint {
                module: MODULE.to_owned(),
                props: "PromoProps".to_owned(),
                contract_hash: HASH,
            }
        );
    }

    #[test]
    fn the_mount_payload_is_nul_free_so_the_cli_can_find_its_end() {
        assert!(!payload(&ENCODED).contains(&0));
    }

    #[test]
    fn the_props_name_is_read_off_the_resolved_schema() {
        assert_eq!(struct_name(&PROMO), "PromoProps");
    }

    #[test]
    fn a_truncated_mount_payload_is_an_error_not_a_guess() {
        let bytes = payload(&ENCODED);
        let error = decode_mount(&bytes[..bytes.len() - 1]).expect_err("a cut hash cannot decode");
        assert!(
            matches!(error, crate::DecodeError::Truncated { .. }),
            "{error}"
        );
    }

    /// A payload that stops after its version byte has no kind byte to read:
    /// it is truncated, not a payload of the default kind.
    #[test]
    fn a_payload_that_ends_after_the_version_is_truncated_for_every_decoder() {
        let version = [crate::FORMAT_VERSION];
        for error in [
            decode_mount(&version).expect_err("no kind byte to be a mount point"),
            crate::decode_catalog(&version).expect_err("no kind byte to be a catalog"),
        ] {
            assert_eq!(error, crate::DecodeError::Truncated { offset: 1 });
        }
    }

    #[test]
    fn trailing_bytes_after_the_hash_are_refused() {
        let mut bytes = payload(&ENCODED).to_vec();
        bytes.push(1);
        assert_eq!(
            decode_mount(&bytes).expect_err("a payload with extra bytes is malformed"),
            crate::DecodeError::Trailing { extra: 1 }
        );
    }
}
