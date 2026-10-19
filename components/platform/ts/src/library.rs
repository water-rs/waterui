//! The JavaScript library's identity: one hash over every file of `src/js`.
//!
//! A bundle is compiled against the library — the signals, the host seam,
//! the JSX runtime — and calls into it by name. The library changes with this
//! crate, and a bundle built against an older one may call an entry that
//! moved. [`LIBRARY_HASH`] is the library half of the runtime fingerprint a
//! bundle's manifest declares and the loader checks, so a bundle for another
//! library is refused before it is cached rather than failing at a call site.
//!
//! The hash is computed by the compiler from the files themselves, in one
//! FNV-1a chain over `<name>\0<contents>\0` per file in the fixed order below,
//! so only the sixty-four bits reach the binary — the sources do not. The
//! `water` CLI never recomputes it: it reads
//! [`waterui_meta_ts_runtime_library`] out of this crate's rlib, which is the
//! same constant, on the channel every other `waterui_meta_*` payload uses.

use waterui_ts_schema::{
    HASH_BASIS, RuntimePart, dir_entry, encode_runtime_half, hash_extend, payload,
    runtime_half_encoded_len,
};

/// Every file of the library, in the order they are hashed.
///
/// Adding a file to `src/js` without listing it here leaves it out of the
/// fingerprint; `tests/js/library.test.js` and the bundle fixtures are built
/// from the same set, and a file the bundler reaches that this list does not
/// is a mistake a code review has to catch — there is no directory walk in a
/// `const` context.
const FILES: [(&str, &[u8]); 7] = [
    ("components.js", include_bytes!("js/components.js")),
    ("contexts.js", include_bytes!("js/contexts.js")),
    ("host.js", include_bytes!("js/host.js")),
    ("index.js", include_bytes!("js/index.js")),
    ("jsx-runtime.js", include_bytes!("js/jsx-runtime.js")),
    ("runtime-global.js", include_bytes!("js/runtime-global.js")),
    ("signals.js", include_bytes!("js/signals.js")),
];

/// One chain over every file: name, NUL, contents, NUL.
const fn hash_files(files: &[(&str, &[u8])]) -> u64 {
    let mut state = HASH_BASIS;
    let mut index = 0;
    while index < files.len() {
        let (name, contents) = files[index];
        state = hash_extend(state, name.as_bytes());
        state = hash_extend(state, &[0]);
        state = hash_extend(state, contents);
        state = hash_extend(state, &[0]);
        index += 1;
    }
    state
}

/// The library half of the runtime fingerprint.
pub const LIBRARY_HASH: u64 = hash_files(&FILES);

/// The library hash, NUL-terminated as the artifact static carries it.
const ENCODED: [u8; runtime_half_encoded_len(RuntimePart::Library, LIBRARY_HASH) + 1] =
    encode_runtime_half(RuntimePart::Library, LIBRARY_HASH);

/// The library half of the runtime fingerprint, read back by the `water` CLI.
///
/// `#[used]` parks it in the `.wmeta` (`__DATA,__wmeta` on Apple) metadata
/// directory as a `name`, NUL, `payload`, NUL record, so the CLI and tests
/// recover it by record name — never by symbol, because a linked image need
/// not carry a symbol table at all (a PE keeps none). The debug gate is
/// what keeps a shipped application free of it.
#[cfg(debug_assertions)]
#[used]
#[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".wmeta"))]
#[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__wmeta"))]
#[expect(
    non_upper_case_globals,
    reason = "tooling names the record by its `waterui_meta_` item name, so the name is the \
              contract"
)]
pub static waterui_meta_ts_runtime_library: [u8; RUNTIME_LIBRARY_RECORD_LEN] =
    dir_entry::<RUNTIME_LIBRARY_RECORD_LEN>(
        b"waterui_meta_ts_runtime_library",
        LIBRARY_HALF_ENCODED,
    );

/// The record's length: the name, NUL, the encoded half, NUL.
const RUNTIME_LIBRARY_RECORD_LEN: usize =
    b"waterui_meta_ts_runtime_library".len() + ENCODED.len() + 1;

/// The encoded library half without its terminator: the exact bytes the CLI
/// recovers from the artifact.
pub const LIBRARY_HALF_ENCODED: &[u8] = payload(&ENCODED);
