//! Compile-level coverage for the `TsProps` expansion's facade path.
//!
//! `#[derive(TsProps)]` names `::waterui::ts` when the consuming crate depends
//! on the facade — `::waterui::ts::schema` for the contract, and the runtime
//! crate re-exported beside it for the conversions. An integration test of
//! `waterui-internal` is such a consumer — the facade is a dev-dependency — so
//! this derive exercises that arm end to end: the emitted path, the facade
//! module it resolves through, the encoded contract, and the conversion the
//! expansion emits next to it.
#![cfg(feature = "ts")]

use waterui::Binding;
use waterui::ts::schema::{TsProps, contract_hash};

/// The props a TypeScript panel module would declare.
#[derive(TsProps)]
struct PanelProps {
    /// Live two-way state, projected as `Signal<number>`.
    volume: Binding<u32>,
    /// A callback back into the shell.
    on_commit: Box<dyn Fn(u32) + Send>,
}

/// The other half of the expansion, which only has to compile: a props struct
/// carrying a callback crosses into TypeScript, and is never read back — which
/// is what lets it carry the callback at all.
const _: fn() = || {
    const fn crosses_into_javascript<T: waterui::ts::IntoJs>() {}
    crosses_into_javascript::<PanelProps>();
};

#[test]
fn ts_props_derives_through_the_facade_module() {
    assert_eq!(
        PanelProps::CONTRACT_HASH,
        contract_hash(PanelProps::ENCODED),
        "the contract hash is read from the encoded payload"
    );
}
