//! Compile-level coverage for the `TsProps` expansion's facade path.
//!
//! `#[derive(TsProps)]` names `::waterui::ts::schema` when the consuming crate
//! depends on the facade. An integration test of `waterui-internal` is such a
//! consumer — the facade is a dev-dependency — so this derive exercises that
//! arm end to end: the emitted path, the facade module it resolves through,
//! and the encoded contract itself.
#![cfg(feature = "ts")]

use waterui::Binding;
use waterui::ts::schema::{TsProps, contract_hash};

/// The props a TypeScript panel module would declare.
#[derive(TsProps)]
#[expect(
    dead_code,
    reason = "the schema is derived from the declaration; nothing constructs the fixture"
)]
struct PanelProps {
    /// Live two-way state, projected as `Signal<number>`.
    volume: Binding<u32>,
    /// A callback back into the shell.
    on_commit: Box<dyn Fn(u32) + Send>,
}

#[test]
fn ts_props_derives_through_the_facade_module() {
    assert_eq!(
        PanelProps::CONTRACT_HASH,
        contract_hash(PanelProps::ENCODED),
        "the contract hash is read from the encoded payload"
    );
}
