//! The checks a bundle passes before it is trusted, in the order they run.
//!
//! Signature first (only a downloaded bundle has one), then the runtime
//! fingerprint, then every module the binary mounts with the contract it was
//! compiled against, then the translation catalog, then the bundle file's
//! digest. Each check answers with a [`Rejection`] naming what it found, and
//! every rejection is logged through `tracing` by the caller that decides what
//! to do about it. Nothing here writes anything.

use suiteki::Str;
use waterui_locale::{Locale, TranslationCatalog};
use waterui_ts_schema::{BundleManifest, ContractHash, RuntimeFingerprint, Sha256Digest};

use super::Requirement;

/// Why a bundle was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Rejection {
    /// The manifest's signature does not verify under the embedded public
    /// key: the manifest was not published by the holder of the private key,
    /// or was altered after it was.
    #[cfg(feature = "ota")]
    #[error("the manifest's signature does not verify under the application's public key")]
    Signature,

    /// The bundle was built for another runtime.
    #[error(
        "the bundle was built for runtime {declared}, and this binary is runtime {expected}: \
         the JavaScript library, the component catalog or the schema format differ"
    )]
    Runtime {
        /// The fingerprint the manifest declares.
        declared: RuntimeFingerprint,
        /// The fingerprint this binary has.
        expected: RuntimeFingerprint,
    },

    /// The bundle does not carry a module the binary mounts.
    #[error("the bundle carries no module \"{id}\", which this binary mounts")]
    MissingModule {
        /// The module the binary mounts.
        id: Str,
    },

    /// A module the binary mounts was built against another props contract.
    #[error(
        "module \"{id}\" was built against props contract {declared}, and this binary mounts it \
         with contract {expected}: the bundle and the binary are from different builds"
    )]
    ContractMismatch {
        /// The module the binary mounts.
        id: Str,
        /// The hash the binary's props type has.
        expected: ContractHash,
        /// The hash the manifest declares.
        declared: ContractHash,
    },

    /// A translation catalog entry is keyed by something that is not a locale.
    #[error("the translation catalog is keyed by {locale:?}, which is not a locale: {reason}")]
    Locale {
        /// The key as the manifest spells it.
        locale: String,
        /// Why it did not parse.
        reason: String,
    },

    /// A translation file does not parse.
    #[error("the translation file for {locale} does not parse: {reason}")]
    Translation {
        /// The locale whose file failed.
        locale: String,
        /// The parser's reason.
        reason: String,
    },

    /// The bundle file's bytes do not hash to what the manifest declares.
    #[error(
        "the bundle file hashes to {found}, not the {expected} its manifest declares: the file \
         was altered or is not the one the manifest was published with"
    )]
    Digest {
        /// The digest the manifest declares.
        expected: Sha256Digest,
        /// The digest of the bytes in hand.
        found: Sha256Digest,
    },

    /// The bundle file is not UTF-8, so it cannot be JavaScript source.
    #[error("the bundle file is not UTF-8 text: {reason}")]
    Encoding {
        /// The decoder's reason.
        reason: String,
    },
}

/// Checks the manifest against what the binary requires and builds the
/// translation catalog it carries.
///
/// The fingerprint is compared first, then every required module, then the
/// translations: a bundle for another runtime is reported as that rather than
/// as a list of missing modules.
///
/// # Errors
///
/// Returns the first [`Rejection`] in that order.
pub fn requirement(
    manifest: &BundleManifest,
    requirement: &Requirement,
) -> Result<Option<TranslationCatalog>, Rejection> {
    if manifest.runtime != requirement.fingerprint {
        return Err(Rejection::Runtime {
            declared: manifest.runtime,
            expected: requirement.fingerprint,
        });
    }
    for module in requirement.modules {
        let declared =
            manifest
                .modules
                .get(module.id)
                .copied()
                .ok_or_else(|| Rejection::MissingModule {
                    id: Str::from(module.id),
                })?;
        if declared.0 != module.contract {
            return Err(Rejection::ContractMismatch {
                id: Str::from(module.id),
                expected: ContractHash(module.contract),
                declared,
            });
        }
    }
    translations(manifest)
}

/// The translation catalog a manifest carries, or `None` when it carries no
/// entries.
///
/// Every key is parsed as a locale before `add_toml` sees it, because
/// `TranslationCatalog` treats an invalid locale as a programming error and
/// panics, and a downloaded document is an input, not a program.
fn translations(manifest: &BundleManifest) -> Result<Option<TranslationCatalog>, Rejection> {
    if manifest.translations.is_empty() {
        return Ok(None);
    }
    let mut catalog = TranslationCatalog::new();
    for (locale, document) in &manifest.translations {
        locale
            .parse::<Locale>()
            .map_err(|error| Rejection::Locale {
                locale: locale.clone(),
                reason: error.to_string(),
            })?;
        catalog = catalog
            .add_toml(locale.clone(), document)
            .map_err(|error| Rejection::Translation {
                locale: locale.clone(),
                reason: error.to_string(),
            })?;
    }
    Ok(Some(catalog))
}

/// Checks the bundle file's bytes against the digest its manifest declares
/// and hands them back as source text.
///
/// # Errors
///
/// [`Rejection::Digest`] when the bytes are not the file the manifest was
/// published with; [`Rejection::Encoding`] when they are not UTF-8.
pub fn bundle<'a>(bytes: &'a [u8], expected: &Sha256Digest) -> Result<&'a str, Rejection> {
    let found = digest(bytes);
    if found != *expected {
        return Err(Rejection::Digest {
            expected: *expected,
            found,
        });
    }
    core::str::from_utf8(bytes).map_err(|error| Rejection::Encoding {
        reason: error.to_string(),
    })
}

/// The SHA-256 digest of `bytes`, as a manifest carries one.
#[must_use]
pub fn digest(bytes: &[u8]) -> Sha256Digest {
    use sha2::Digest as _;
    Sha256Digest::new(sha2::Sha256::digest(bytes).into())
}

/// Checks the manifest's signature under `key`.
///
/// The bytes verified are [`BundleManifest::signed_bytes`] of the manifest as
/// parsed — never the bytes as downloaded — so whitespace and member order in
/// the served file do not matter, and a document that parses to the same
/// manifest verifies whatever it looked like.
///
/// # Errors
///
/// [`Rejection::Signature`] when it does not verify. `verify_strict` is the
/// check: it refuses the non-canonical signature encodings that a lenient
/// verifier accepts, so there is exactly one signature for one document.
#[cfg(feature = "ota")]
pub fn signature(
    signed: &waterui_ts_schema::SignedManifest,
    key: &ed25519_dalek::VerifyingKey,
) -> Result<(), Rejection> {
    let signature = ed25519_dalek::Signature::from_bytes(signed.signature.as_bytes());
    key.verify_strict(&signed.manifest.signed_bytes(), &signature)
        .map_err(|_| Rejection::Signature)
}
