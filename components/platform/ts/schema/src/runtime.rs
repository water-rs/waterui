//! The runtime fingerprint: which runtime a bundle was built for.
//!
//! A bundle is JavaScript compiled against two things the binary carries: the
//! JavaScript library `waterui-ts` embeds (`src/js/*.js`, the signals, the
//! host seam, the JSX runtime) and the component catalog the facade publishes
//! (which tags JSX may name and what each accepts). Change either and a bundle
//! built before the change may call an entry that no longer exists or hand a
//! component an attribute it no longer takes. The fingerprint is what a
//! bundle's manifest declares and what a binary checks it against, so a bundle
//! for another runtime is refused before it is cached.
//!
//! # The two halves and the artifact
//!
//! The library hash is computed by `waterui-ts`, the crate that owns the
//! JavaScript; the catalog hash is computed by the facade, the crate that owns
//! the vocabulary. Neither can see the other, and the `water` CLI must read
//! both without re-reading any source, so each crate emits its half as a
//! `#[cfg(debug_assertions)] #[used] static` — `waterui_meta_ts_runtime_library`
//! and `waterui_meta_ts_runtime_catalog` — on the same channel as the props
//! contracts, the catalog and the mount points. [`encode_runtime_half`] writes
//! one, [`decode_runtime_half`] reads it back, and [`RuntimePart`] says which
//! half it is.
//!
//! The CLI combines the halves with [`RuntimeFingerprint::new`] and writes
//! [`RuntimeFingerprint`]'s text form into the bundle manifest; the binary
//! combines the same two constants with the same constructor at compile time,
//! so the value the loader compares against at launch is the value the CLI
//! derived from the artifacts of that very build.
//!
//! # Text form
//!
//! `<format version>-<library hash>-<catalog hash>`, each hash as sixteen
//! lowercase hexadecimal digits: `2-9f86d081884c7d65-9b71d224bd62f378`. The
//! format version is [`FORMAT_VERSION`], the first byte of every payload the
//! CLI reads, so a bundle built with one generation of the schema format is
//! never mistaken for one built with another.

use core::fmt;
use core::str::FromStr;

use crate::decode::{DecodeError, Reader};
use crate::encode::{put, put_u64};
use crate::format::{FORMAT_VERSION, kind, payload_kind, runtime_part};

/// Which half of the runtime fingerprint a payload carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RuntimePart {
    /// The JavaScript library embedded in `waterui-ts`.
    Library,
    /// The component catalog the facade publishes.
    Catalog,
}

impl RuntimePart {
    /// The byte that names this half in a payload.
    const fn tag(self) -> u8 {
        match self {
            Self::Library => runtime_part::LIBRARY,
            Self::Catalog => runtime_part::CATALOG,
        }
    }
}

/// One half of the runtime fingerprint, recovered from an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RuntimeHalf {
    /// Which half this is.
    pub part: RuntimePart,
    /// The hash that half carries.
    pub hash: u64,
}

/// Write the whole half into `buf`, returning the length.
const fn put_runtime_half(buf: &mut [u8], part: RuntimePart, hash: u64) -> usize {
    let pos = put(buf, 0, FORMAT_VERSION);
    let pos = put(buf, pos, kind::RUNTIME_HALF);
    let pos = put(buf, pos, part.tag());
    put_u64(buf, pos, hash)
}

/// Length in bytes of a runtime half's encoded payload, not counting the NUL
/// terminator the artifact static appends.
#[must_use]
pub const fn runtime_half_encoded_len(part: RuntimePart, hash: u64) -> usize {
    let mut probe: [u8; 0] = [];
    put_runtime_half(&mut probe, part, hash)
}

/// Encode one half of the runtime fingerprint into a NUL-terminated array.
///
/// `N` must be `runtime_half_encoded_len(part, hash) + 1`, for the same
/// reason [`encode`](crate::encode) requires it of a props contract: the CLI
/// finds a static's end at its first NUL, because a Mach-O symbol carries no
/// size.
///
/// # Panics
/// Fails const evaluation when `N` is not
/// `runtime_half_encoded_len(part, hash) + 1`.
#[must_use]
pub const fn encode_runtime_half<const N: usize>(part: RuntimePart, hash: u64) -> [u8; N] {
    let mut encoded = [0_u8; N];
    let end = put_runtime_half(&mut encoded, part, hash);
    assert!(
        end + 1 == N,
        "encode_runtime_half::<N> requires N == runtime_half_encoded_len(part, hash) + 1"
    );
    encoded
}

/// Decode a runtime-half payload.
///
/// The payload is the byte range between the version byte and the NUL
/// terminator, exactly as [`decode`](crate::decode) takes a props contract.
///
/// # Errors
/// Returns a [`DecodeError`] for an unknown version, a payload of another
/// kind, an unknown part byte, a truncated payload, or bytes left over after
/// the hash.
pub fn decode_runtime_half(payload: &[u8]) -> Result<RuntimeHalf, DecodeError> {
    if payload.is_empty() {
        return Err(DecodeError::Empty);
    }
    let mut reader = Reader {
        bytes: payload,
        pos: 0,
        depth: 0,
    };
    let version = reader.byte()?;
    if version != FORMAT_VERSION {
        return Err(DecodeError::Version {
            found: version,
            expected: FORMAT_VERSION,
        });
    }
    if payload.get(reader.pos) != Some(&kind::RUNTIME_HALF) {
        return Err(DecodeError::NotARuntimeHalf {
            found: payload_kind(payload.get(reader.pos).copied()),
        });
    }
    reader.pos += 1;

    let offset = reader.pos;
    let part = match reader.byte()? {
        runtime_part::LIBRARY => RuntimePart::Library,
        runtime_part::CATALOG => RuntimePart::Catalog,
        tag => {
            return Err(DecodeError::UnknownTag {
                kind: "runtime part",
                tag,
                offset,
            });
        }
    };
    let hash = reader.hash()?;

    let extra = payload.len() - reader.pos;
    if extra > 0 {
        return Err(DecodeError::Trailing { extra });
    }
    Ok(RuntimeHalf { part, hash })
}

/// The runtime a bundle was built for: the schema format version, the
/// JavaScript library's hash and the component catalog's hash.
///
/// Built with [`new`](Self::new) from the two halves — by the binary at
/// compile time from its own constants, and by the CLI from the two artifact
/// statics — and compared whole: a bundle whose manifest declares a different
/// fingerprint targets another runtime and is refused. It serializes as its
/// text form, which is what the manifest carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeFingerprint {
    format: u8,
    library: u64,
    catalog: u64,
}

impl RuntimeFingerprint {
    /// Combines the two halves under the format version this crate implements.
    #[must_use]
    pub const fn new(library: u64, catalog: u64) -> Self {
        Self {
            format: FORMAT_VERSION,
            library,
            catalog,
        }
    }

    /// The format version the fingerprint was built under.
    #[must_use]
    pub const fn format(self) -> u8 {
        self.format
    }

    /// The JavaScript library's hash.
    #[must_use]
    pub const fn library(self) -> u64 {
        self.library
    }

    /// The component catalog's hash.
    #[must_use]
    pub const fn catalog(self) -> u64 {
        self.catalog
    }
}

impl fmt::Display for RuntimeFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}-{:016x}-{:016x}",
            self.format, self.library, self.catalog
        )
    }
}

/// Why a fingerprint's text form could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FingerprintParseError {
    /// The text does not have the three dash-separated parts.
    #[error(
        "a runtime fingerprint is `<format>-<library hash>-<catalog hash>`, and {found:?} is not"
    )]
    Shape {
        /// The text that was read.
        found: String,
    },
    /// The format version is not a decimal byte.
    #[error("the format version {found:?} of a runtime fingerprint is not a decimal byte")]
    Format {
        /// The text of the version part.
        found: String,
    },
    /// A hash is not exactly sixteen lowercase hexadecimal digits.
    #[error(
        "the {which} hash {found:?} of a runtime fingerprint is not sixteen lowercase \
         hexadecimal digits"
    )]
    Hash {
        /// Which hash: `library` or `catalog`.
        which: &'static str,
        /// The text of the hash part.
        found: String,
    },
}

/// Reads exactly sixteen lowercase hexadecimal digits.
pub fn parse_hex64(text: &str) -> Option<u64> {
    if text.len() != 16
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    u64::from_str_radix(text, 16).ok()
}

impl FromStr for RuntimeFingerprint {
    type Err = FingerprintParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut parts = text.split('-');
        let (Some(format), Some(library), Some(catalog), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(FingerprintParseError::Shape {
                found: text.to_owned(),
            });
        };
        let format = format
            .parse::<u8>()
            .map_err(|_| FingerprintParseError::Format {
                found: format.to_owned(),
            })?;
        let library = parse_hex64(library).ok_or_else(|| FingerprintParseError::Hash {
            which: "library",
            found: library.to_owned(),
        })?;
        let catalog = parse_hex64(catalog).ok_or_else(|| FingerprintParseError::Hash {
            which: "catalog",
            found: catalog.to_owned(),
        })?;
        Ok(Self {
            format,
            library,
            catalog,
        })
    }
}

impl serde::Serialize for RuntimeFingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for RuntimeFingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}
