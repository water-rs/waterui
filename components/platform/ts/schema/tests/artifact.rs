//! The channel end to end: derive a props contract, then read it back out of
//! the symbol table of the binary this test is running from.
//!
//! This is the whole point of the crate. The contract has to survive const
//! evaluation of a deeply nested tree, `#[used]` retention through the linker,
//! and the CLI's recovery step — cut the static's section data at its first
//! NUL, because a Mach-O symbol carries no size. The recovery below mirrors
//! `ArtifactSymbols::static_bytes` in the `water` CLI so a divergence between
//! the two shows up here rather than in a project build.

#![cfg(feature = "waterui")]

use std::collections::{BTreeMap, BTreeSet};

use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
use waterui_core::{AnyView, Binding, Computed};
use waterui_ts_schema::{TsProps, TsType, TypeSchema, contract_hash, decode, owned, payload};

/// A unit-only enum: a union of string literals.
#[derive(TsType)]
#[expect(
    dead_code,
    reason = "a schema is derived from the declaration; the fixtures are never constructed"
)]
enum Tier {
    Free,
    Pro,
    Team,
}

/// A data-carrying enum: a tagged object, with one variant of each payload shape.
#[derive(TsType)]
#[expect(
    dead_code,
    reason = "a schema is derived from the declaration; the fixtures are never constructed"
)]
enum Slot {
    Empty,
    Badge(u32),
    Note { text: String, pinned: bool },
}

/// Level three.
#[derive(TsType)]
#[expect(
    dead_code,
    reason = "a schema is derived from the declaration; the fixtures are never constructed"
)]
struct Profile {
    handle: String,
    tier: Tier,
    quota: u64,
}

/// Level two.
#[derive(TsType)]
#[expect(
    dead_code,
    reason = "a schema is derived from the declaration; the fixtures are never constructed"
)]
struct Panel {
    profile: Profile,
    slots: Vec<Slot>,
    labels: BTreeMap<String, String>,
}

/// Level one: the root contract, carrying the `Binding<Vec<Enum>>` that made
/// const evaluation of this tree the open question.
#[derive(TsProps)]
#[expect(
    dead_code,
    reason = "a schema is derived from the declaration; the fixtures are never constructed"
)]
struct PromoProps {
    panel: Panel,
    history: Binding<Vec<Slot>>,
    seen: Computed<u32>,
    banner: AnyView,
    on_dismiss: Box<dyn Fn(u32)>,
    note: Option<String>,
}

/// The leaf segment of a demangled symbol name.
fn leaf_of(name: &str) -> Option<&str> {
    name.rsplit("::").next().filter(|leaf| !leaf.is_empty())
}

/// Demangle a raw symbol name, dropping the trailing disambiguation hash and
/// the leading underscore Mach-O adds to unmangled names.
fn demangled_name(raw: &str) -> String {
    let demangled = format!("{:#}", rustc_demangle::demangle(raw));
    demangled
        .strip_prefix('_')
        .map_or_else(|| demangled.clone(), str::to_owned)
}

/// Bytes of the `#[used] static` whose demangled leaf is `leaf`, read from the
/// running executable exactly the way the CLI reads them from an rlib.
fn meta_static(leaf: &str) -> Vec<u8> {
    let path = std::env::current_exe().expect("the test binary has a path");
    let data = std::fs::read(&path).expect("the test binary is readable");
    let file = object::File::parse(&*data).expect("the test binary parses as an object file");
    let mut payloads = BTreeSet::new();
    for symbol in file.symbols() {
        let Ok(raw) = symbol.name() else { continue };
        if leaf_of(&demangled_name(raw)) != Some(leaf) {
            continue;
        }
        let Some(index) = symbol.section_index() else {
            continue;
        };
        let Ok(section) = file.section_by_index(index) else {
            continue;
        };
        let Ok(section_data) = section.data() else {
            continue;
        };
        let Ok(offset) = usize::try_from(symbol.address().wrapping_sub(section.address())) else {
            continue;
        };
        if let Some(bytes) = section_data.get(offset..) {
            payloads.insert(
                bytes
                    .split(|byte| *byte == 0)
                    .next()
                    .unwrap_or_default()
                    .to_vec(),
            );
        }
    }
    let mut payloads = payloads.into_iter();
    let found = payloads
        .next()
        .unwrap_or_else(|| panic!("no symbol with leaf `{leaf}` carries section data"));
    assert!(
        payloads.next().is_none(),
        "`{leaf}` is defined more than once with different payloads"
    );
    found
}

#[test]
fn the_contract_reaches_the_artifact_and_decodes_to_the_const_schema() {
    let bytes = meta_static("waterui_meta_tsprops_PromoProps");
    assert_eq!(
        bytes,
        PromoProps::ENCODED,
        "the artifact payload is the constant the compiler encoded"
    );
    assert_eq!(
        decode(&bytes).expect("the artifact payload decodes"),
        owned::Schema::from(&PromoProps::SCHEMA),
        "decoding the artifact payload rebuilds the const schema"
    );
    assert_eq!(contract_hash(&bytes), PromoProps::CONTRACT_HASH);
}

#[test]
fn the_nested_tree_is_resolved_through_four_levels() {
    let TypeSchema::Struct(root) = PromoProps::SCHEMA else {
        panic!("the root is a struct")
    };
    let history = root
        .fields
        .iter()
        .find(|field| field.name == "history")
        .expect("the props carry `history`");
    let TypeSchema::Signal(list) = history.ty else {
        panic!("`Binding<_>` is a signal, got {}", history.ty)
    };
    let TypeSchema::List(item) = *list else {
        panic!("`Vec<_>` is a list, got {list}")
    };
    let TypeSchema::Enum(slot) = *item else {
        panic!("`Slot` is an enum, got {item}")
    };
    assert_eq!(slot.name, "Slot");
    assert_eq!(slot.variants.len(), 3);

    let panel = root
        .fields
        .iter()
        .find(|field| field.name == "panel")
        .expect("the props carry `panel`");
    let TypeSchema::Struct(panel) = panel.ty else {
        panic!("`Panel` is a struct")
    };
    let TypeSchema::Struct(profile) = panel.fields[0].ty else {
        panic!("`Profile` is a struct")
    };
    let TypeSchema::Enum(tier) = profile.fields[1].ty else {
        panic!("`Tier` is an enum")
    };
    assert_eq!(
        tier.representation,
        waterui_ts_schema::EnumRepresentation::StringUnion,
        "an enum whose variants are all unit variants is a string union"
    );
}

#[test]
fn the_payload_is_nul_free_so_the_cli_can_find_its_end() {
    assert!(!PromoProps::ENCODED.contains(&0));
    assert_eq!(payload(&[1, 0]), &[1]);
}
