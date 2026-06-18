//! Shared helpers for validating and preparing RGBA pixel uploads.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;

#[derive(Debug, Clone, Copy)]
#[allow(
    clippy::enum_variant_names,
    reason = "the `Overflow` suffix documents that each variant is an overflow condition"
)]
pub(crate) enum PixelUploadError {
    BytesPerRowOverflow,
    DataLengthOverflow,
    AlignmentOverflow,
}

impl core::fmt::Display for PixelUploadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BytesPerRowOverflow => write!(f, "bytes_per_row overflow"),
            Self::DataLengthOverflow => write!(f, "pixel data length overflow"),
            Self::AlignmentOverflow => write!(f, "aligned bytes_per_row overflow"),
        }
    }
}

enum UploadBytes<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

pub(crate) struct PreparedRgbaUpload<'a> {
    bytes: UploadBytes<'a>,
    bytes_per_row: u32,
}

impl PreparedRgbaUpload<'_> {
    #[must_use]
    pub(crate) fn bytes(&self) -> &[u8] {
        match &self.bytes {
            UploadBytes::Borrowed(bytes) => bytes,
            UploadBytes::Owned(bytes) => bytes.as_slice(),
        }
    }

    #[must_use]
    pub(crate) const fn bytes_per_row(&self) -> u32 {
        self.bytes_per_row
    }
}

/// Prepares raw RGBA8 pixel data for `wgpu::Queue::write_texture`.
///
/// Ensures byte count math does not overflow and row stride satisfies wgpu's
/// `COPY_BYTES_PER_ROW_ALIGNMENT` requirement.
///
/// # Safety
///
/// `input_handle` must point to at least `width * height * 4` valid bytes.
pub(crate) unsafe fn prepare_rgba8_upload<'a>(
    input_handle: *mut c_void,
    width: u32,
    height: u32,
) -> Result<PreparedRgbaUpload<'a>, PixelUploadError> {
    let bytes_per_row = width
        .checked_mul(4)
        .ok_or(PixelUploadError::BytesPerRowOverflow)?;
    let data_len = bytes_per_row
        .checked_mul(height)
        .ok_or(PixelUploadError::DataLengthOverflow)?;
    let src = unsafe {
        core::slice::from_raw_parts(input_handle.cast_const().cast::<u8>(), data_len as usize)
    };

    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padding = (alignment - (bytes_per_row % alignment)) % alignment;
    let aligned_bytes_per_row = bytes_per_row
        .checked_add(padding)
        .ok_or(PixelUploadError::AlignmentOverflow)?;

    if padding == 0 {
        return Ok(PreparedRgbaUpload {
            bytes: UploadBytes::Borrowed(src),
            bytes_per_row,
        });
    }

    let aligned_len = aligned_bytes_per_row
        .checked_mul(height)
        .ok_or(PixelUploadError::DataLengthOverflow)?;
    let mut staged = vec![0_u8; aligned_len as usize];
    let src_stride = bytes_per_row as usize;
    let dst_stride = aligned_bytes_per_row as usize;

    for row in 0..height as usize {
        let src_offset = row * src_stride;
        let dst_offset = row * dst_stride;
        staged[dst_offset..dst_offset + src_stride]
            .copy_from_slice(&src[src_offset..src_offset + src_stride]);
    }

    Ok(PreparedRgbaUpload {
        bytes: UploadBytes::Owned(staged),
        bytes_per_row: aligned_bytes_per_row,
    })
}
