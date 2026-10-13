//! Mount points: which TypeScript modules a binary mounts, and against which
//! props contract.
//!
//! A props contract says what one module receives; the catalog says what JSX
//! may name. This is the third payload the format carries, and it answers the
//! remaining question: *which* modules does this application actually mount?
//! `tsx!("./promo.tsx", PromoProps { … })` emits one of these in a
//! `waterui_meta_tsx_*` `#[used] static`, and the `water` CLI reads them back
//! out of a dev-profile host build to learn the module list it has to bundle
//! and the contract each module must be typed against.
//!
//! Three facts travel, all of them produced by the compiler rather than by
//! reading source text:
//!
//! * the module id — the `.tsx` file's path relative to the crate's
//!   `CARGO_MANIFEST_DIR`, with forward slashes and the extension kept, which
//!   is the key the bundle publishes the module under;
//! * the props type's name, taken from that type's own
//!   [`TypeSchema::Struct`] constant, so a mount written through a type alias
//!   still records the struct the derive was written on;
//! * the props contract hash, [`TsProps::CONTRACT_HASH`](crate::TsProps),
//!   which joins the mount to its `waterui_meta_tsprops_*` schema exactly and
//!   is the same number a bundle's per-module manifest declares.

use crate::decode::{DecodeError, Reader};
use crate::encode::{put, put_str, put_u64};
use crate::format::{FORMAT_VERSION, kind, payload_kind};
use crate::tree::TypeSchema;

/// The name of the struct `schema` describes.
///
/// This is how a `tsx!` expansion records which props type a module is typed
/// against without reading the token it was spelled with: the name comes from
/// the type's own [`TypeSchema`] constant, which the compiler resolved through
/// aliases and generic arguments first.
///
/// # Panics
/// Fails const evaluation when the schema is not a struct, which a props
/// contract's always is.
#[must_use]
pub const fn struct_name(schema: &TypeSchema) -> &'static str {
    match schema {
        TypeSchema::Struct(schema) => schema.name,
        _ => panic!("a props contract is declared as a struct with named fields"),
    }
}

/// Write the whole mount point into `buf`, returning the length.
const fn put_mount(buf: &mut [u8], module: &str, props: &str, contract_hash: u64) -> usize {
    assert!(!module.is_empty(), "a module id must not be empty");
    assert!(!props.is_empty(), "a props type name must not be empty");
    let pos = put(buf, 0, FORMAT_VERSION);
    let pos = put(buf, pos, kind::MOUNT);
    let pos = put_str(buf, pos, module);
    let pos = put_str(buf, pos, props);
    put_u64(buf, pos, contract_hash)
}

/// Length in bytes of a mount point's encoded payload, not counting the NUL
/// terminator the artifact static appends.
///
/// # Panics
/// Fails const evaluation on the same violations [`encode_mount`] rejects.
#[must_use]
pub const fn mount_encoded_len(module: &str, props: &str, contract_hash: u64) -> usize {
    let mut probe: [u8; 0] = [];
    put_mount(&mut probe, module, props, contract_hash)
}

/// Encode a mount point into a NUL-terminated array.
///
/// `N` must be `mount_encoded_len(module, props, contract_hash) + 1`, for the
/// same reason [`encode`](crate::encode) requires it of a props contract: the
/// CLI finds a static's end at its first NUL, because a Mach-O symbol carries
/// no size.
///
/// # Panics
/// Fails const evaluation when `N` is not
/// `mount_encoded_len(module, props, contract_hash) + 1`, or when the module
/// id or the props type name is empty.
#[must_use]
pub const fn encode_mount<const N: usize>(
    module: &str,
    props: &str,
    contract_hash: u64,
) -> [u8; N] {
    let mut encoded = [0_u8; N];
    let end = put_mount(&mut encoded, module, props, contract_hash);
    assert!(
        end + 1 == N,
        "encode_mount::<N> requires N == mount_encoded_len(module, props, contract_hash) + 1"
    );
    encoded
}

/// One mount point, recovered from an artifact.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MountPoint {
    /// The module id: the `.tsx` path relative to the crate's manifest
    /// directory, forward-slashed, extension kept.
    pub module: String,
    /// The name of the props struct the module is typed against.
    pub props: String,
    /// The props contract hash, which the bundle's manifest must match.
    pub contract_hash: u64,
}

/// Decode a mount-point payload.
///
/// The payload is the byte range between the version byte and the NUL
/// terminator, exactly as [`decode`](crate::decode) takes a props contract.
///
/// # Errors
/// Returns a [`DecodeError`] for an unknown version, a payload of another
/// kind, a truncated payload, an empty module id or props type name, or bytes
/// left over after the hash.
pub fn decode_mount(payload: &[u8]) -> Result<MountPoint, DecodeError> {
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
    // Read rather than peeked: a payload that ends after the version byte is
    // truncated, not a payload of another kind.
    let found = reader.byte()?;
    if found != kind::MOUNT {
        return Err(DecodeError::NotAMountPoint {
            found: payload_kind(found),
        });
    }

    let module = named(&mut reader, "module")?;
    let props = named(&mut reader, "props")?;
    let contract_hash = reader.hash()?;

    let extra = payload.len() - reader.pos;
    if extra > 0 {
        return Err(DecodeError::Trailing { extra });
    }
    Ok(MountPoint {
        module,
        props,
        contract_hash,
    })
}

/// Read one name, which the encoder guarantees is not empty.
fn named(reader: &mut Reader<'_>, what: &'static str) -> Result<String, DecodeError> {
    let offset = reader.pos;
    let name = reader.string()?;
    if name.is_empty() {
        return Err(DecodeError::EmptyName { kind: what, offset });
    }
    Ok(name)
}
