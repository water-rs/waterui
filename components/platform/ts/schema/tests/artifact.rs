//! The channel end to end: derive a props contract, then read it back out of
//! the metadata directory section of the binary this test is running from.
//!
//! This is the whole point of the crate. The contract has to survive const
//! evaluation of a deeply nested tree, `#[used]` retention through the
//! linker, and the CLI's recovery step — walk the `.wmeta` section's
//! `name`, NUL, `payload`, NUL records and match the name. The section
//! channel is what every format shares: a linked image need not keep a
//! symbol table at all — a linked PE carries none — so the record names
//! itself instead of relying on one. The recovery below mirrors
//! `ArtifactSymbols::static_bytes` in the `water` CLI so a divergence
//! between the two shows up here rather than in a project build.

#![cfg(feature = "waterui")]

use std::collections::{BTreeMap, BTreeSet};

use object::{Object as _, ObjectSection as _};
use waterui_core::{AnyView, Binding, Computed};
use waterui_ts_schema::{
    DIR_SECTION, TsProps, TsType, TypeSchema, contract_hash, decode, dir_records, owned, payload,
};

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

/// Payloads the `.wmeta` (`__wmeta` on Mach-O) metadata directory holds for
/// `leaf`. The section is the whole channel — it survives into a linked
/// image on every format, while a symbol table may not exist at all (a
/// linked PE carries none), so each record names itself.
fn dir_payloads(file: &object::File<'_>, leaf: &str) -> BTreeSet<Vec<u8>> {
    let mut payloads = BTreeSet::new();
    for section in file.sections() {
        let Ok(name) = section.name() else { continue };
        if !matches!(name, DIR_SECTION | "__wmeta") {
            continue;
        }
        let Ok(data) = section.data() else { continue };
        for record in dir_records(data) {
            if record.name == leaf.as_bytes() {
                payloads.insert(record.payload.to_vec());
            }
        }
    }
    payloads
}

/// Bytes of the `#[used] static` whose record names `leaf`, read from the
/// running executable's metadata directory exactly the way the CLI reads
/// them.
fn meta_static(leaf: &str) -> Vec<u8> {
    let path = std::env::current_exe().expect("the test binary has a path");
    let data = std::fs::read(&path).expect("the test binary is readable");
    let file = object::File::parse(&*data).expect("the test binary parses as an object file");
    let mut payloads = dir_payloads(&file, leaf).into_iter();
    let found = payloads
        .next()
        .unwrap_or_else(|| panic!("no metadata record named `{leaf}` in the directory section"));
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
