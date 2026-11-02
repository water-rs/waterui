//! `meta-retention` — three `dir_static!` records in three visibilities,
//! plus the marker `main` black-boxes so the object carrying them links in.
//!
//! Staged into an owned directory by the `apple_release_metadata_retention`
//! test, which serializes the manifest itself: this crate ships only
//! sources.

use waterui_meta::dir_entry;

const ROOT_NAME: &[u8] = b"waterui_meta_retention_root";
const ROOT_PAYLOAD: &[u8] = b"root record";
const ROOT_LEN: usize = ROOT_NAME.len() + ROOT_PAYLOAD.len() + 2;

waterui_meta::dir_static! {
    /// The crate-root public record.
    #[allow(non_upper_case_globals)]
    pub static waterui_meta_retention_root: [u8; ROOT_LEN] =
        dir_entry::<ROOT_LEN>(ROOT_NAME, ROOT_PAYLOAD);
}

mod private {
    use waterui_meta::dir_entry;

    const NAME: &[u8] = b"waterui_meta_retention_private";
    const PAYLOAD: &[u8] = b"private module record";
    const LEN: usize = NAME.len() + PAYLOAD.len() + 2;

    waterui_meta::dir_static! {
        /// A `pub` record whose owning module is private.
        #[allow(non_upper_case_globals)]
        pub static waterui_meta_retention_private: [u8; LEN] =
            dir_entry::<LEN>(NAME, PAYLOAD);
    }
}

const _: () = {
    use waterui_meta::dir_entry;

    const NAME: &[u8] = b"waterui_meta_retention_anon";
    const PAYLOAD: &[u8] = b"anonymous const record";
    const LEN: usize = NAME.len() + PAYLOAD.len() + 2;

    waterui_meta::dir_static! {
        /// A fully private record inside an anonymous const.
        #[allow(non_upper_case_globals)]
        static waterui_meta_retention_anon: [u8; LEN] =
            dir_entry::<LEN>(NAME, PAYLOAD);
    }
};

/// Referenced from the fixture binary through `black_box`, so the object
/// carrying the records is linked without the records themselves being
/// read at runtime.
#[inline(never)]
pub fn live_marker() -> u8 {
    7
}
