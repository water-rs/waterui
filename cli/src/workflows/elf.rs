//! ELF `PT_LOAD` alignment checks for packaged shared libraries.
//!
//! Android 15 devices with 16 KB page sizes — and Google Play's 2025
//! requirement — reject a package in which any shared library maps a `LOAD`
//! segment with an alignment below the page size. Every `.so` the CLI stages
//! into an Android package is checked here so a misaligned artifact fails the
//! build naming the file, rather than surfacing on the device as an install
//! error or a compatibility dialog.

use std::path::Path;

use eyre::bail;

/// The alignment every packaged `LOAD` segment must reach: 16 KB, the largest
/// page size Android ships with.
pub const REQUIRED_LOAD_ALIGNMENT: u64 = 0x4000;

/// Require every `PT_LOAD` segment in the ELF at `path` to declare an
/// alignment of at least [`REQUIRED_LOAD_ALIGNMENT`].
///
/// An unreadable or unparsable file is an error: a staged library that cannot
/// be inspected cannot be trusted to load.
///
/// # Errors
/// Returns an error when the file cannot be read or parsed as ELF, or when
/// any `PT_LOAD` segment's alignment is below the requirement.
pub fn require_aligned_load_segments(path: &Path) -> eyre::Result<()> {
    let data = std::fs::read(path).map_err(|error| {
        eyre::eyre!(
            "failed to read staged library {} for ELF alignment validation: {error}",
            path.display()
        )
    })?;
    check_load_segment_alignment(&data)
        .map_err(|error| eyre::eyre!("staged library {}: {error}", path.display()))
}

/// Require every `*.so` directly inside `directory` to satisfy
/// [`require_aligned_load_segments`].
///
/// # Errors
/// Returns an error when the directory cannot be read or any library inside
/// fails the alignment check.
pub async fn require_aligned_shared_libraries(directory: &Path) -> eyre::Result<()> {
    let directory = directory.to_path_buf();
    smol::unblock(move || {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.extension() != Some(std::ffi::OsStr::new("so")) {
                continue;
            }
            require_aligned_load_segments(&path)?;
        }
        Ok(())
    })
    .await
}

/// `Ok(())` when every `PT_LOAD` segment of the ELF image is aligned to at
/// least [`REQUIRED_LOAD_ALIGNMENT`]; an error naming the offending segment
/// otherwise.
fn check_load_segment_alignment(data: &[u8]) -> eyre::Result<()> {
    use object::read::elf::{ElfFile, FileHeader, ProgramHeader as _};

    fn scan<Elf>(data: &[u8]) -> eyre::Result<()>
    where
        Elf: FileHeader<Endian = object::Endianness>,
    {
        let file = ElfFile::<Elf>::parse(data)
            .map_err(|error| eyre::eyre!("not a parseable ELF file: {error}"))?;
        let endian = file.endian();
        let headers = file
            .elf_header()
            .program_headers(endian, data)
            .map_err(|error| eyre::eyre!("cannot read ELF program headers: {error}"))?;
        for header in headers {
            if header.p_type(endian) != object::elf::PT_LOAD {
                continue;
            }
            let align: u64 = header.p_align(endian).into();
            if align < REQUIRED_LOAD_ALIGNMENT {
                let offset: u64 = header.p_offset(endian).into();
                bail!(
                    "PT_LOAD segment (offset {offset:#x}) is aligned to {align:#x}; \
                     Android 16 KB page-size support requires at least {REQUIRED_LOAD_ALIGNMENT:#x}",
                );
            }
        }
        Ok(())
    }

    match data.get(4) {
        Some(&object::elf::ELFCLASS64) => {
            scan::<object::elf::FileHeader64<object::Endianness>>(data)
        }
        Some(&object::elf::ELFCLASS32) => {
            scan::<object::elf::FileHeader32<object::Endianness>>(data)
        }
        _ => bail!("not a parseable ELF file: missing or invalid ELF class byte"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        REQUIRED_LOAD_ALIGNMENT, check_load_segment_alignment, require_aligned_load_segments,
    };

    /// Minimal ELF64 header + one program header with a chosen `p_align`,
    /// enough for `object` to enumerate segments.
    fn elf_with_load_align(align: u64) -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[..4].copy_from_slice(b"\x7fELF");
        data[4] = 2; // ELFCLASS64
        data[5] = 1; // little-endian
        data[6] = 1; // EI_VERSION (EV_CURRENT)
        data[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
        data[18..20].copy_from_slice(&0xb7u16.to_le_bytes()); // EM_AARCH64
        data[20..24].copy_from_slice(&1u32.to_le_bytes()); // version
        data[32..40].copy_from_slice(&64u64.to_le_bytes()); // e_phoff
        data[52..54].copy_from_slice(&64u16.to_le_bytes()); // e_ehsize
        data[54..56].copy_from_slice(&56u16.to_le_bytes()); // e_phentsize
        data[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        // program header at offset 64
        data[64..68].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        data[68..72].copy_from_slice(&5u32.to_le_bytes()); // flags
        data[72..80].copy_from_slice(&0u64.to_le_bytes()); // p_offset
        data[112..120].copy_from_slice(&align.to_le_bytes()); // p_align
        data
    }

    #[test]
    fn accepts_a_16k_aligned_load_segment() {
        check_load_segment_alignment(&elf_with_load_align(REQUIRED_LOAD_ALIGNMENT))
            .expect("16 KB-aligned LOAD must pass");
        check_load_segment_alignment(&elf_with_load_align(0x10000))
            .expect("larger alignment must pass");
    }

    #[test]
    fn rejects_a_4k_aligned_load_segment() {
        let error = check_load_segment_alignment(&elf_with_load_align(0x1000))
            .expect_err("4 KB-aligned LOAD must fail");
        let message = error.to_string();
        assert!(
            message.contains("0x1000"),
            "message names the alignment: {message}"
        );
        assert!(
            message.contains("0x4000"),
            "message names the requirement: {message}"
        );
    }

    #[test]
    fn rejects_non_elf_data() {
        let _ = check_load_segment_alignment(b"not an elf").expect_err("non-ELF must fail");
    }

    #[test]
    fn require_aligned_load_segments_reports_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("libbad.so");
        std::fs::write(&path, elf_with_load_align(0x1000)).expect("write");
        let error = require_aligned_load_segments(&path).expect_err("must fail");
        assert!(error.to_string().contains("libbad.so"));
    }
}
