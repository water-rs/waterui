//! The bundle manifest: what a published bundle says about itself, and the
//! exact bytes its signature covers.
//!
//! One document travels three ways. The `water` CLI writes a
//! [`BundleManifest`] beside every bundle it builds; the one built into the
//! application ships inside the binary as the baseline, unsigned because the
//! binary's own signature already covers it; the one published for download
//! is wrapped in a [`SignedManifest`], whose ed25519 signature the runtime
//! checks with the public key the application embeds. The runtime verifies
//! every bundle — baseline or downloaded — against the same manifest type
//! through the same checks, so there is one verification path and one
//! document to get right.
//!
//! # The document
//!
//! ```json
//! {
//!   "manifest": {
//!     "version": 3,
//!     "runtime": "2-9f86d081884c7d65-9b71d224bd62f378",
//!     "bundle": { "url": "bundle-3.js", "size": 48213, "sha256": "<64 hex digits>" },
//!     "modules": { "src/views/promo.tsx": "0123456789abcdef" },
//!     "translations": { "en": "greeting = \"Hello\"\n" }
//!   },
//!   "signature": "<128 hex digits>"
//! }
//! ```
//!
//! The baseline embeds the inner `manifest` object alone. Every hash is
//! lowercase hexadecimal of fixed width: a contract hash is sixteen digits,
//! the same spelling `installRuntimeGlobal`'s `contracts` table uses; a SHA-256
//! digest is sixty-four; an ed25519 signature is one hundred and
//! twenty-eight. `translations` maps a locale tag to the TOML text of that
//! locale's translation file — the same document `TranslationCatalog::add_toml`
//! takes — and is omitted when the bundle carries none. `bundle.url` is the
//! bundle's location relative to the manifest's own URL; for the baseline it
//! is the file name the CLI wrote beside it. `bundle.size` is the bundle
//! file's length in bytes: the bound the client reads the download under,
//! signed so that a server cannot make the client buffer more than the
//! publisher shipped.
//!
//! # The signed bytes
//!
//! The signature covers [`BundleManifest::signed_bytes`]: the compact JSON
//! serialization of the inner `manifest` object, exactly as `serde_json`
//! writes this type. Spelled out, so another implementation can reproduce it
//! byte for byte: UTF-8, no whitespace anywhere, the members `version`,
//! `runtime`, `bundle`, `modules` and (only when non-empty) `translations` in
//! that order, `bundle`'s members `url`, `size` then `sha256`, the entries of
//! `modules` and `translations` sorted by key as byte strings, integers in
//! plain decimal, and strings quoted with `\"`, `\\`, `\n`, `\r`, `\t`, `\b`
//! and `\f` as two-character escapes, every other control character as
//! `\u00XX` with lowercase hex, and nothing else escaped. The bundle file's
//! bytes are covered through their digest in `bundle.sha256`, which is why a
//! manifest can be checked before the bundle is downloaded and a bundle can be
//! checked with nothing but the manifest in hand.
//!
//! The verifier never trusts the bytes it downloaded to be canonical: it
//! parses the document, re-serializes the parsed manifest, and checks the
//! signature over that. Whitespace, member order and escaping in the file as
//! served therefore do not matter; only the content does.

use std::collections::BTreeMap;

use crate::runtime::RuntimeFingerprint;

/// Fixed-width bytes carried as lowercase hexadecimal text.
///
/// The width is part of the type, so a digest that is one digit short is a
/// parse error naming the width rather than a shorter digest that never
/// matches.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HexBytes<const N: usize>([u8; N]);

impl<const N: usize> HexBytes<N> {
    /// Wraps the bytes.
    #[must_use]
    pub const fn new(bytes: [u8; N]) -> Self {
        Self(bytes)
    }

    /// The bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; N] {
        &self.0
    }

    /// Reads exactly `2 * N` lowercase hexadecimal digits.
    fn parse(text: &str) -> Result<Self, HexError> {
        let expected = 2 * N;
        if text.len() != expected {
            return Err(HexError::Width {
                expected,
                found: text.len(),
            });
        }
        let mut bytes = [0_u8; N];
        for (index, pair) in text.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            let high = nibble(pair[0]).ok_or(HexError::Digit { offset: index * 2 })?;
            let low = nibble(pair[1]).ok_or(HexError::Digit {
                offset: index * 2 + 1,
            })?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

/// One lowercase hexadecimal digit's value.
const fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

/// Why hexadecimal text could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HexError {
    /// The text is not the width the type requires.
    #[error("expected {expected} hexadecimal digits, found {found} characters")]
    Width {
        /// The number of digits the type requires.
        expected: usize,
        /// The number of characters found.
        found: usize,
    },
    /// A character is not a lowercase hexadecimal digit.
    #[error("the character at offset {offset} is not a lowercase hexadecimal digit")]
    Digit {
        /// Where the character sits.
        offset: usize,
    },
}

impl<const N: usize> core::fmt::Display for HexBytes<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl<const N: usize> core::fmt::Debug for HexBytes<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "HexBytes<{N}>({self})")
    }
}

impl<const N: usize> core::str::FromStr for HexBytes<N> {
    type Err = HexError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl<const N: usize> From<[u8; N]> for HexBytes<N> {
    fn from(bytes: [u8; N]) -> Self {
        Self(bytes)
    }
}

impl<const N: usize> serde::Serialize for HexBytes<N> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de, const N: usize> serde::Deserialize<'de> for HexBytes<N> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A SHA-256 digest: the bundle file's identity inside its manifest.
pub type Sha256Digest = HexBytes<32>;

/// An ed25519 signature over [`BundleManifest::signed_bytes`].
pub type SignatureBytes = HexBytes<64>;

/// A props contract hash as a manifest carries it: sixteen lowercase
/// hexadecimal digits, the spelling the bundle's own `contracts` table uses.
///
/// The value is [`TsProps::CONTRACT_HASH`](crate::TsProps::CONTRACT_HASH). It
/// is text in the document because it is 64 bits wide and JSON numbers are
/// not reliably that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractHash(pub u64);

impl core::fmt::Display for ContractHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

impl serde::Serialize for ContractHash {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ContractHash {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        crate::runtime::parse_hex64(&text).map(Self).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "a contract hash is sixteen lowercase hexadecimal digits, and {text:?} is not"
            ))
        })
    }
}

/// Where the bundle file is, how long it is, and what it hashes to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleFile {
    /// The bundle's location, relative to the manifest's own URL.
    pub url: String,
    /// The bundle file's length in bytes. A download is read under this
    /// bound: a response that declares or delivers more is refused without
    /// being buffered, so the signed manifest — not the server — decides how
    /// much memory a fetch may take.
    pub size: u64,
    /// The SHA-256 digest of the bundle file's bytes.
    pub sha256: Sha256Digest,
}

/// What a bundle says about itself.
///
/// Serialized in declaration order; [`signed_bytes`](Self::signed_bytes) is
/// the canonical form the module documentation spells out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    /// The bundle version. A larger number is newer: the loader prefers the
    /// newest verified bundle, and a published bundle is only ever preferred
    /// to the baseline when its version is greater than the baseline's.
    pub version: u64,
    /// The runtime the bundle was built for.
    pub runtime: RuntimeFingerprint,
    /// The bundle file.
    pub bundle: BundleFile,
    /// Every module the bundle carries, with the props contract hash each was
    /// built against.
    pub modules: BTreeMap<String, ContractHash>,
    /// The translation catalog the bundle ships, as locale tag to the TOML
    /// text of that locale's translation file. Empty when it ships none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, String>,
}

impl BundleManifest {
    /// The bytes the signature covers: this manifest's compact JSON form.
    ///
    /// Deterministic by construction — the type serializes its members in
    /// declaration order, maps are `BTreeMap`s, and `serde_json` writes one
    /// canonical compact form — so the signer and the verifier agree on every
    /// byte without agreeing on anything but this type.
    ///
    /// # Panics
    ///
    /// Never in practice: every field is a plain value `serde_json` always
    /// serializes, so the only failure `to_vec` has is unreachable here.
    #[must_use]
    pub fn signed_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self)
            .expect("a bundle manifest serializes: every field is a plain value")
    }

    /// Reads a manifest from its JSON text.
    ///
    /// # Errors
    /// Returns the `serde_json` error when the text is not a manifest.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The manifest as pretty-printed JSON, for a file a person may read.
    ///
    /// # Panics
    ///
    /// Never in practice, for the reason [`signed_bytes`](Self::signed_bytes)
    /// gives.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .expect("a bundle manifest serializes: every field is a plain value")
    }
}

/// A manifest as published for download: the manifest and its signature.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedManifest {
    /// The manifest the signature covers.
    pub manifest: BundleManifest,
    /// The ed25519 signature over [`BundleManifest::signed_bytes`].
    pub signature: SignatureBytes,
}

impl SignedManifest {
    /// Reads a signed manifest from its JSON text.
    ///
    /// # Errors
    /// Returns the `serde_json` error when the text is not a signed manifest.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// The signed manifest as pretty-printed JSON, which is the file the CLI
    /// publishes.
    ///
    /// # Panics
    ///
    /// Never in practice, for the reason
    /// [`BundleManifest::signed_bytes`] gives.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .expect("a signed manifest serializes: every field is a plain value")
    }
}
