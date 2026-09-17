//! TypeScript-facing surface: the runtime, the component vocabulary it builds
//! views from, and the props contract both are typed against.
//!
//! `waterui-ts` is re-exported wholesale, so `waterui::ts::Bridge`,
//! `waterui::ts::TsRuntime` and `waterui::ts::schema` name the runtime crate's
//! items and a `TsType`/`TsProps` derive on a crate that consumes the facade
//! finds every item the expansion uses.
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
