//! Artifact linker-section channels: `.wmeta` metadata records and `.wreach`
//! reachability markers.
//!
//! Two channels live here because neither can rely on a shipped binary's
//! symbol table (a linked PE image keeps no COFF symbol table at all, and
//! release builds set `strip = true`):
//!
//! - **Metadata directory (`.wmeta`)**: a proc macro that conveys metadata to
//!   tooling (the `water` CLI, a test, a packager) emits a `#[used] static`
//!   whose item name starts with `waterui_meta_`, parks it in `.wmeta` on
//!   COFF and ELF (`__DATA,__wmeta` for an Apple debug build and
//!   `__DWARF,__wmeta,regular,debug` for an Apple release build), and makes
//!   its bytes `name`, NUL, `payload`, NUL. `__DWARF` content is stripped
//!   with the rest of the debug data when `ld` links a final image, so a
//!   release binary ships no `.wmeta` record while its rlib still carries one.
//! - **Reachability markers (`.wreach`)**: each `NativeBackend` realization
//!   entry and the erased dispatch entry reference, through
//!   [`core::hint::black_box`], a `static` marker in `.wreach`
//!   (`__DATA,__wreach` on Apple) carrying the realization's NUL-terminated
//!   name and **no** `#[used]`. Because the section stays in the final image
//!   while the static itself is not forced live, LTO inlining preserves a
//!   reached marker's reference, `strip = true` leaves section bytes intact,
//!   and `--gc-sections` / `-dead_strip` / `/OPT:REF` removes a marker
//!   exactly when it removes the code that references it.
//!
//! `no_std` plus `alloc`, with zero dependencies.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

/// The section name the metadata directory is parked in on COFF and ELF
/// targets.
///
/// On Mach-O, readers match `__wmeta` regardless of its segment: `__DATA`
/// in debug and `__DWARF` in release.
pub const DIR_SECTION: &str = ".wmeta";

/// Builds one directory record — `name`, NUL, `payload`, NUL — in const
/// context for an emitted static's bytes.
///
/// The record ends with the payload's own terminator, so runs of NUL in the
/// section are only the terminators and any padding the linker inserted
/// between records — a reader skips leading NUL bytes, reads the name, then
/// the payload.
///
/// # Panics
/// Fails const evaluation when `M` is not `name.len() + payload.len() + 2`,
/// so a record can never truncate a name or a payload.
#[must_use]
pub const fn dir_entry<const M: usize>(name: &[u8], payload: &[u8]) -> [u8; M] {
    assert!(
        M == name.len() + payload.len() + 2,
        "a metadata directory record is `name`, NUL, `payload`, NUL"
    );
    let mut record = [0u8; M];
    let mut i = 0;
    while i < name.len() {
        record[i] = name[i];
        i += 1;
    }
    let mut i = 0;
    while i < payload.len() {
        record[name.len() + 1 + i] = payload[i];
        i += 1;
    }
    record
}

/// The same record [`dir_entry`] builds, as owned bytes: macros that
/// already know their payload at expansion time bake it into a literal,
/// while emitted consts use [`dir_entry`].
#[must_use]
pub fn dir_record(name: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(name.len() + payload.len() + 2);
    record.extend_from_slice(name);
    record.push(0);
    record.extend_from_slice(payload);
    record.push(0);
    record
}

/// One record read out of a metadata directory: the name a
/// `waterui_meta_*` item carries, and the payload it describes, each cut at
/// its first NUL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirRecord<'a> {
    /// The record's name — the `waterui_meta_`-prefixed item name.
    pub name: &'a [u8],
    /// The record's payload bytes.
    pub payload: &'a [u8],
}

/// An iterator over the records in one metadata directory's section bytes.
///
/// Runs of NUL are skipped before each record — they are only the previous
/// record's terminator and any padding the linker inserted — and a
/// truncated record ends iteration rather than guessing at bytes it cannot
/// frame.
#[derive(Debug, Clone)]
pub struct DirRecords<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for DirRecords<'a> {
    type Item = DirRecord<'a>;

    fn next(&mut self) -> Option<DirRecord<'a>> {
        let first = self.rest.iter().position(|byte| *byte != 0)?;
        self.rest = &self.rest[first..];
        let end = self.rest.iter().position(|byte| *byte == 0)?;
        let (name, tail) = self.rest.split_at(end);
        let tail = &tail[1..];
        let end = tail.iter().position(|byte| *byte == 0)?;
        let (payload, rest) = tail.split_at(end);
        self.rest = &rest[1..];
        Some(DirRecord { name, payload })
    }
}

/// Walks the records of one `.wmeta` (`__wmeta` on Mach-O) section's bytes.
///
/// The section lookup itself stays with the caller's object reader: an
/// artifact may be an object file, an archive, or a linked image, and each
/// reader already knows how to find its sections.
#[must_use]
pub const fn dir_records(data: &[u8]) -> DirRecords<'_> {
    DirRecords { rest: data }
}

/// Emits a `waterui_meta_*` record static with the placement and retention
/// the *invoking* crate's own target and profile call for.
///
/// The emitted static keeps the caller's attributes, visibility, `[u8; N]`
/// type and value verbatim and adds, at the invocation site — the profile
/// the record lands under is the consumer's, not this crate's:
///
/// - on Apple targets, `#[used]` plus `__DATA,__wmeta` under
///   `debug_assertions`, or `__DWARF,__wmeta,regular,debug` in a release
///   build — `#[used]` keeps the static in the object, and the DWARF
///   section flag is what lets `ld` strip it from the shipped image;
/// - on COFF and ELF targets, `.wmeta` plus `#[used]` — the attribute keeps
///   the record in the object even when nothing references it (a bare
///   `pub static` survives codegen only while something does), and the
///   linker's ordinary dead strip still removes it from a linked image.
#[macro_export]
macro_rules! dir_static {
    ($(#[$attr:meta])* $vis:vis static $name:ident: [u8; $len:expr] = $value:expr;) => {
        $(#[$attr])*
        #[cfg_attr(
            all(target_vendor = "apple", debug_assertions),
            unsafe(link_section = "__DATA,__wmeta")
        )]
        #[cfg_attr(
            all(target_vendor = "apple", not(debug_assertions)),
            unsafe(link_section = "__DWARF,__wmeta,regular,debug")
        )]
        #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".wmeta"))]
        #[used]
        $vis static $name: [u8; $len] = $value;
    };
}

/// The section name reachability markers are parked in on COFF and ELF
/// targets.
///
/// On Mach-O, readers match `__wreach` in `__DATA` (see [`is_reach_section`]).
pub const REACH_SECTION: &str = ".wreach";

/// Whether `name` is the reachability marker section on any supported object
/// format (`.wreach` on COFF and ELF, `__wreach` on Mach-O).
#[must_use]
pub fn is_reach_section(name: &str) -> bool {
    matches!(name, REACH_SECTION | "__wreach")
}

/// Builds one reachability marker record — `name`, NUL — in const context
/// for an emitted static's bytes.
///
/// The record ends with its own terminator, so runs of NUL in the section
/// are only the terminators and any padding the linker inserted between
/// records.
///
/// # Panics
/// Fails const evaluation when `name` is empty, contains a NUL byte, or `M`
/// is not `name.len() + 1`.
#[must_use]
pub const fn reach_entry<const M: usize>(name: &[u8]) -> [u8; M] {
    assert!(
        !name.is_empty(),
        "a reachability record name cannot be empty"
    );
    assert!(M == name.len() + 1, "a reachability record is `name`, NUL");
    let mut record = [0u8; M];
    let mut i = 0;
    while i < name.len() {
        assert!(
            name[i] != 0,
            "a reachability record name cannot contain a NUL byte"
        );
        record[i] = name[i];
        i += 1;
    }
    record
}

/// The same record [`reach_entry`] builds, as owned bytes.
///
/// # Panics
/// Panics when `name` is empty or contains a NUL byte.
#[must_use]
pub fn reach_record(name: &[u8]) -> Vec<u8> {
    assert!(
        !name.is_empty(),
        "a reachability record name cannot be empty"
    );
    assert!(
        !name.contains(&0),
        "a reachability record name cannot contain a NUL byte"
    );
    let mut record = Vec::with_capacity(name.len() + 1);
    record.extend_from_slice(name);
    record.push(0);
    record
}

/// An iterator over the NUL-terminated realization names in one `.wreach`
/// (`__wreach` on Mach-O) section's bytes.
///
/// Runs of NUL are skipped before each record — they are only the previous
/// record's terminator and any padding the linker inserted — and a
/// truncated record without a terminating NUL ends iteration rather than
/// guessing at bytes it cannot frame.
#[derive(Debug, Clone)]
pub struct ReachRecords<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for ReachRecords<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        let first = self.rest.iter().position(|byte| *byte != 0)?;
        self.rest = &self.rest[first..];
        let end = self.rest.iter().position(|byte| *byte == 0)?;
        let (name, tail) = self.rest.split_at(end);
        self.rest = &tail[1..];
        Some(name)
    }
}

/// Walks the records of one `.wreach` (`__wreach` on Mach-O) section's bytes.
#[must_use]
pub const fn reach_records(data: &[u8]) -> ReachRecords<'_> {
    ReachRecords { rest: data }
}

/// Emits a reachability marker static in `.wreach` (`__DATA,__wreach` on
/// Apple targets) **without** `#[used]`.
///
/// Because the static carries no `#[used]` while the section stays in the
/// linked image, linker garbage collection (`--gc-sections`, `-dead_strip`,
/// `/OPT:REF`) drops the marker if and only if the code referencing it is
/// removed.
#[macro_export]
macro_rules! reach_static {
    ($(#[$attr:meta])* $vis:vis static $name:ident: [u8; $len:expr] = $value:expr;) => {
        $(#[$attr])*
        #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__wreach"))]
        #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".wreach"))]
        $vis static $name: [u8; $len] = $value;
    };
}

/// Marks the enclosing realization or dispatch entry as reached when its
/// code survives into the linked artifact.
///
/// Emits a NUL-terminated `name` record via [`reach_static!`] and references
/// its address through [`core::hint::black_box`], so LTO inlining keeps the
/// reference while dead-code elimination drops both the code and the marker
/// together.
#[macro_export]
macro_rules! mark_reached {
    ($name:expr) => {{
        const NAME_STR: &str = $name;
        const NAME: &[u8] = NAME_STR.as_bytes();
        const LEN: usize = NAME.len() + 1;
        $crate::reach_static! {
            static MARKER: [u8; LEN] = $crate::reach_entry::<LEN>(NAME);
        }
        ::core::hint::black_box(&MARKER);
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_entry_and_dir_record_agree() {
        const ENTRY: [u8; 19] = dir_entry::<19>(b"waterui_meta_x", b"abc");
        assert_eq!(ENTRY, *b"waterui_meta_x\0abc\0");
        assert_eq!(dir_record(b"waterui_meta_x", b"abc"), ENTRY.to_vec());
    }

    #[test]
    fn dir_static_builds_a_walkable_record() {
        const NAME: &[u8] = b"waterui_meta_test_dir_static";
        const PAYLOAD: &[u8] = b"dir_static probe";
        const LEN: usize = NAME.len() + PAYLOAD.len() + 2;
        crate::dir_static! {
            /// The `dir_static!` round-trip record.
            #[allow(non_upper_case_globals)]
            static waterui_meta_test_dir_static: [u8; LEN] =
                dir_entry::<LEN>(NAME, PAYLOAD);
        }
        let record = dir_records(&waterui_meta_test_dir_static)
            .next()
            .expect("the emitted static carries one record");
        assert_eq!(record.name, NAME);
        assert_eq!(record.payload, PAYLOAD);
    }

    #[test]
    fn dir_records_walks_a_padded_section() {
        // Records joined by linker padding (runs of NUL) walk one by one.
        let mut section = dir_record(b"waterui_meta_a", b"p1");
        section.extend_from_slice(&[0, 0, 0]);
        section.extend_from_slice(&dir_record(b"waterui_meta_b", b"p2"));
        let records: Vec<_> = dir_records(&section).collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].name, b"waterui_meta_a");
        assert_eq!(records[0].payload, b"p1");
        assert_eq!(records[1].name, b"waterui_meta_b");
        assert_eq!(records[1].payload, b"p2");
    }

    #[test]
    fn dir_records_stops_at_a_truncated_record() {
        // A name with no payload terminator yields nothing rather than
        // reading past the section's end.
        let mut section = dir_record(b"waterui_meta_a", b"p1");
        section.extend_from_slice(b"waterui_meta_b\0");
        let records: Vec<_> = dir_records(&section).collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, b"waterui_meta_a");
    }

    #[test]
    fn dir_records_skips_a_nul_only_section() {
        assert_eq!(dir_records(&[0, 0, 0]).next(), None);
        assert_eq!(dir_records(&[]).next(), None);
    }

    #[test]
    fn reach_entry_and_reach_record_agree() {
        const ENTRY: [u8; 7] = reach_entry::<7>(b"button");
        assert_eq!(ENTRY, *b"button\0");
        assert_eq!(reach_record(b"button"), ENTRY.to_vec());
    }

    #[test]
    fn reach_static_and_mark_reached_build_walkable_records() {
        const NAME: &[u8] = b"test_reach_static";
        const LEN: usize = NAME.len() + 1;
        crate::reach_static! {
            static MARKER: [u8; LEN] = reach_entry::<LEN>(NAME);
        }
        crate::mark_reached!("test_mark_reached");
        assert_eq!(reach_records(&MARKER).collect::<Vec<_>>(), [NAME]);
    }

    #[test]
    fn reach_records_walks_a_padded_section_and_stops_at_truncation() {
        let mut section = reach_record(b"text");
        section.extend_from_slice(&[0, 0, 0]);
        section.extend_from_slice(&reach_record(b"button"));
        section.extend_from_slice(b"truncated_without_nul");
        let records: Vec<_> = reach_records(&section).collect();
        assert_eq!(records, [&b"text"[..], &b"button"[..]]);
        assert_eq!(reach_records(&[0, 0, 0]).next(), None);
        assert_eq!(reach_records(&[]).next(), None);
    }

    #[test]
    fn is_reach_section_matches_elf_coff_and_macho_names() {
        assert!(is_reach_section(".wreach"));
        assert!(is_reach_section("__wreach"));
        assert!(!is_reach_section(".wmeta"));
        assert!(!is_reach_section("__wmeta"));
    }
}
