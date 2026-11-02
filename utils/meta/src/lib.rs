//! The metadata-directory record format every `waterui_meta_*` artifact
//! channel shares.
//!
//! A proc macro that must convey metadata to tooling (the `water` CLI, a
//! test, a packager) emits a `static` whose item name starts with
//! `waterui_meta_`, parks it in a dedicated linker section — `.wmeta` on
//! COFF and ELF, `__DATA,__wmeta` for an Apple debug build and
//! `__DWARF,__wmeta,regular,debug` for an Apple release build — and makes
//! its bytes a self-describing record: the item's name, NUL, the payload,
//! NUL.
//!
//! The two Apple placements split retention: `__DWARF` content is stripped
//! with the rest of the debug data when `ld` links a final image, so a
//! release binary ships no record while its rlib still carries one, and a
//! debug build's `__DATA,__wmeta` keeps the record in the linked image a
//! harness reads.
//!
//! The section exists because a symbol table cannot be trusted to: a linked
//! PE image keeps no COFF symbol table at all (the header's symbol fields
//! are deprecated and linkers write them as zero), so a static findable
//! only by symbol name is unreachable there — the record's own name field
//! is what identifies it, in objects, rlibs, and linked images alike.
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
/// - on COFF and ELF targets, `.wmeta` and nothing else — the macro adds
///   no retention attribute there, so the caller's own visibility decides
///   whether the compiler keeps the static in the object; the linker's
///   ordinary dead strip removes the record from a linked image.
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
        #[cfg_attr(target_vendor = "apple", used)]
        $vis static $name: [u8; $len] = $value;
    };
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
}
