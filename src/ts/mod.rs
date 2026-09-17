//! TypeScript-facing surface: the runtime, the component vocabulary it builds
//! views from, and the props contract both are typed against.
//!
//! `waterui-ts` is re-exported wholesale, so `waterui::ts::Bridge`,
//! `waterui::ts::TsRuntime`, `waterui::ts::Mount` and `waterui::ts::schema`
//! name the runtime crate's items and a `TsType`/`TsProps` derive on a crate
//! that consumes the facade finds every item the expansion uses.
//!
//! An application mounts a module with [`tsx!`](crate::tsx), which resolves
//! the module id from the path written beside the Rust file and expands to
//! [`Mount`]; the runtime it mounts into is the one the application's bundle
//! loader installed in the environment as a [`RuntimeHandle`].
//!
//! The bundle a launch runs comes from the [`Loader`], and the fingerprint it
//! verifies bundles against is [`RUNTIME_FINGERPRINT`], assembled here because
//! this is where both halves are in reach. With the `ts-ota` feature the
//! update client (`Ota`, `BundleStore`) is here too.
//!
//! What lives *here* rather than in that crate is the vocabulary: the
//! [`catalog`] of components a JSX tag may name and the [`Components`] host
//! table that builds them. The facade is the one crate that reaches every
//! component and composer `WaterUI` offers — `List` and `Card` are defined in
//! it, and `opacity` and `shadow` are its `ViewExt` methods — and the runtime
//! crate cannot depend back on it, so the vocabulary belongs on this side of
//! that edge. The runtime keeps what has no vocabulary in it: the engines, the
//! bridge, the conversions, the [`HostTable`] trait and the mount scope.

pub use waterui_ts::*;

pub mod catalog;
mod components;

pub use components::Components;

/// The runtime fingerprint of this build: the JavaScript library
/// `waterui-ts` embeds and the component [`catalog`] this crate publishes,
/// under the schema format version.
///
/// This is the value the CLI-generated leaf crate writes into its
/// [`Requirement`], and the value the `water` CLI derives from the two
/// artifact statics `waterui_meta_ts_runtime_library` and
/// `waterui_meta_ts_runtime_catalog` when it builds a bundle's manifest. Both
/// are the same two constants combined by the same constructor, so a bundle
/// the CLI built for this build verifies against this binary, and one built
/// for any other does not.
pub const RUNTIME_FINGERPRINT: schema::RuntimeFingerprint =
    schema::RuntimeFingerprint::new(LIBRARY_HASH, catalog::CATALOG_HASH);

/// `crate::ts` is the path the derives emit for expansions inside
/// `waterui-internal`; deriving here exercises that arm, so a typo in it
/// fails this crate's own tests.
#[cfg(test)]
mod tests {
    use super::schema::{TsProps, contract_hash};
    use crate::Binding;

    /// Props crossing to a mounted TypeScript view.
    #[derive(TsProps)]
    struct SidebarProps {
        unread: Binding<u32>,
    }

    #[test]
    fn ts_props_derives_through_the_internal_crate_path() {
        assert_eq!(
            SidebarProps::CONTRACT_HASH,
            contract_hash(SidebarProps::ENCODED)
        );
    }
}
