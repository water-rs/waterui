//! Detects whether this process's code signature can hold a keychain ACL.
//!
//! Chromium's `OSCrypt` encrypts the persistent cookie store with the
//! "Chromium Safe Storage" login-keychain item — a single global item
//! whose access list is bound to whichever binary created it. The binding
//! that decides whether a later access prompts `SecurityAgent` is the
//! item's `partition_id` ACL entry, and the partition a binary gets comes
//! from its code signature:
//!
//! - A signature carrying a team identifier (Developer ID, Apple
//!   Development) also carries an `application-identifier` entitlement,
//!   so the binary joins a stable `teamid:` partition — the real keychain
//!   prompts at most once (when the item is owned by a different CEF
//!   embedder; the service name is a compile-time `kDefaultServiceName`
//!   in Chromium's `components/os_crypt/keychain_password_mac.mm` and CEF
//!   exposes no override — chromiumembedded/cef#2692).
//! - Any signature without a team identifier — ad-hoc, unsigned, or
//!   self-signed — gets a `cdhash:` partition, which changes on every
//!   rebuild (`water package` re-signs ad-hoc), so every rebuilt binary
//!   prompts again. (`keychain-access-groups`/`application-identifier`
//!   entitlements that would stabilize the partition are restricted and
//!   AMFI rejects them without a provisioning profile.) Only that case
//!   may fall back to `--use-mock-keychain`.

use std::ptr::NonNull;

use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFType};
use objc2_security::{
    SecCSFlags, SecCode, SecCodeSignatureFlags, SecStaticCode, kSecCSSigningInformation,
    kSecCodeInfoFlags, kSecCodeInfoTeamIdentifier,
};

/// Keys are `CFString` and values are arbitrary `CFType`s.
type SigningInformation = CFDictionary<CFType, CFType>;

/// The process's own signing-information dictionary, requested with
/// `kSecCSSigningInformation` — without that flag `kSecCodeInfoTeamIdentifier`
/// is never returned (`SecCode.h` documents it under "Signing"). `None`
/// means the kernel holds no signature for this process, which is itself
/// the "no stable identity" case.
fn signing_information() -> Option<CFRetained<CFDictionary>> {
    // SAFETY: every out-parameter is a live `Option<CFRetained<T>>` slot
    // (same layout as the `T*` the API writes) and only read on success.
    unsafe {
        let mut code: Option<CFRetained<SecCode>> = None;
        if SecCode::copy_self(SecCSFlags::DefaultFlags, NonNull::from(&mut code).cast()) != 0 {
            return None;
        }
        let code = code?;
        let mut static_code: Option<CFRetained<SecStaticCode>> = None;
        if code.copy_static_code(
            SecCSFlags::DefaultFlags,
            NonNull::from(&mut static_code).cast(),
        ) != 0
        {
            return None;
        }
        let static_code = static_code?;
        let mut info: Option<CFRetained<CFDictionary>> = None;
        if SecCode::copy_signing_information(
            &static_code,
            SecCSFlags::from_bits_retain(kSecCSSigningInformation),
            NonNull::from(&mut info).cast(),
        ) != 0
        {
            return None;
        }
        info
    }
}

/// Whether this process must use Chromium's mock keychain because its code
/// signature cannot hold a keychain item ACL across rebuilds.
///
/// Returns `false` only for a signature that carries a team identifier and
/// is not ad-hoc; every other case — ad-hoc, unsigned, or self-signed —
/// gets a `cdhash:` keychain partition and prompts on each rebuild.
#[must_use]
pub fn needs_mock_keychain() -> bool {
    let Some(info) = signing_information() else {
        tracing::info!("CEF keychain: mock (process carries no signing information)");
        return true;
    };
    // SAFETY: the signing dictionary's keys are `CFString`s and its values
    // are `CFType`s of per-key documented classes.
    let info: &SigningInformation = unsafe { info.cast_unchecked() };
    // A properly signed binary always reports a flags word; its absence
    // means the signature is unreadable and cannot anchor an ACL either.
    // SAFETY: the `kSecCodeInfo*` extern statics are immutable SDK
    // constants initialized by the Security framework.
    let adhoc = unsafe {
        info.get(kSecCodeInfoFlags.as_ref())
            .and_then(|flags| flags.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64))
            .is_none_or(|flags| flags & i64::from(SecCodeSignatureFlags::Adhoc.bits()) != 0)
    };
    // SAFETY: same immutable extern static argument as above.
    let has_team_identifier = unsafe { info.get(kSecCodeInfoTeamIdentifier.as_ref()).is_some() };
    let mock = adhoc || !has_team_identifier;
    tracing::info!(
        adhoc,
        has_team_identifier,
        "CEF keychain: {}",
        if mock { "mock" } else { "system" }
    );
    mock
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_security::kSecCodeInfoIdentifier;

    /// `cargo test` binaries are linker-signed ad-hoc on arm64 macOS, so the
    /// decision is exercisable without packaging an app.
    #[test]
    fn unsigned_test_binary_gets_the_mock_keychain() {
        let info = signing_information().expect("cargo test binary is signed");
        // SAFETY: see `needs_mock_keychain`.
        let info: &SigningInformation = unsafe { info.cast_unchecked() };
        // `kSecCodeInfoIdentifier` is only present in the dictionary when
        // `kSecCSSigningInformation` was requested — this proves the request
        // asked for signing information rather than default flags only.
        // SAFETY: `kSecCodeInfoIdentifier` is an immutable SDK constant.
        let has_identifier = unsafe { info.get(kSecCodeInfoIdentifier.as_ref()).is_some() };
        assert!(
            has_identifier,
            "signing dictionary lacks signing information"
        );
        assert!(needs_mock_keychain());
    }
}
