//! On-disk cache of the oracle's `f64` reference images.
//!
//! A corpus render pass recomputes every scene's `f64` oracle image even
//! though the oracle depends only on the scene itself — `scene.json`
//! plus the blobs under `resources/`. `cherenkov-bench reference` renders
//! each scene's [`cherenkov_oracle::Image`] once into `<scene>.ref` and a
//! `render --reference DIR` pass loads the identical pixels instead of
//! re-rendering, so the six corpus passes share one oracle computation.
//!
//! Each `.ref` file carries a fingerprint of the scene's inputs; a load
//! whose fingerprint does not match the scene on disk fails rather than
//! comparing against a stale reference.

use std::io::Write;
use std::path::{Path, PathBuf};

use cherenkov_oracle::Image;

use crate::BenchError;
use crate::convert::Blobs;

const MAGIC: &[u8; 8] = b"CHRREF01";

/// Where the reference for `scene` lives inside a cache directory.
#[must_use]
pub fn path(dir: &Path, scene: &str) -> PathBuf {
    dir.join(format!("{scene}.ref"))
}

/// FNV-1a over the oracle's inputs: the raw `scene.json` bytes, then each
/// referenced resource blob's name and bytes in `Blobs` order.
#[must_use]
pub fn fingerprint(scene_json: &[u8], blobs: &Blobs) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    mix(scene_json);
    for (hash, bytes) in blobs {
        mix(hash.file_name().as_bytes());
        mix(&bytes.len().to_le_bytes());
        mix(bytes);
    }
    h
}

/// Write `image` as `dir`'s `<scene>.ref`, atomically (a partial file
/// never looks like a valid reference).
///
/// # Errors
/// `std::io::Error` on filesystem failures, or an image that does not
/// fit the `u32` dimensions field.
pub fn write(dir: &Path, scene: &str, fingerprint: u64, image: &Image) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let dim = |n: usize| {
        u32::try_from(n).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "image too large to cache")
        })
    };
    let mut buf = Vec::with_capacity(24 + image.pixels.len() * 32);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&fingerprint.to_le_bytes());
    buf.extend_from_slice(&dim(image.width)?.to_le_bytes());
    buf.extend_from_slice(&dim(image.height)?.to_le_bytes());
    for p in &image.pixels {
        for c in p {
            buf.extend_from_slice(&c.to_le_bytes());
        }
    }
    let tmp = dir.join(format!(".{scene}.ref.tmp"));
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        f.write_all(&buf)?;
        f.flush()?;
    }
    std::fs::rename(tmp, path(dir, scene))
}

/// Read and verify `dir`'s `<scene>.ref`.
///
/// # Errors
/// [`BenchError::Engine`] when the file is missing, malformed, or its
/// fingerprint or dimensions do not match `fingerprint` / `expect`;
/// `std::io::Error` on filesystem failures.
pub fn read(
    dir: &Path,
    scene: &str,
    fingerprint: u64,
    expect: (usize, usize),
) -> Result<Image, BenchError> {
    let path = path(dir, scene);
    let buf = std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BenchError::Engine(format!(
                "no cached reference at {}; build it with `cherenkov-bench reference`",
                path.display()
            ))
        } else {
            BenchError::Io(e)
        }
    })?;
    let bad = |why: &str| BenchError::Engine(format!("{}: {why}", path.display()));
    if buf.len() < 24 || &buf[..8] != MAGIC {
        return Err(bad("not a reference file"));
    }
    let fp = u64::from_le_bytes(buf[8..16].try_into().map_err(|_| bad("truncated header"))?);
    if fp != fingerprint {
        return Err(bad("scene inputs changed since the reference was built"));
    }
    let width = u32::from_le_bytes(
        buf[16..20]
            .try_into()
            .map_err(|_| bad("truncated header"))?,
    ) as usize;
    let height = u32::from_le_bytes(
        buf[20..24]
            .try_into()
            .map_err(|_| bad("truncated header"))?,
    ) as usize;
    if (width, height) != expect {
        return Err(bad("reference dimensions do not match the scene"));
    }
    let (pixels_data, rest) = buf[24..].as_chunks::<32>();
    if !rest.is_empty() || pixels_data.len() != width * height {
        return Err(bad("truncated pixel data"));
    }
    let mut pixels = Vec::with_capacity(width * height);
    for px in pixels_data {
        let mut channel = [0f64; 4];
        for (c, bytes) in channel.iter_mut().zip(px.as_chunks::<8>().0) {
            *c = f64::from_le_bytes(*bytes);
        }
        pixels.push(channel);
    }
    Ok(Image {
        width,
        height,
        pixels,
    })
}
