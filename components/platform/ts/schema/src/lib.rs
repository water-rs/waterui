//! The props contract between a `WaterUI` Rust shell and a TypeScript view
//! module, carried into the compiled artifact.
//!
//! Rust owns the contract: a props struct declares what a mounted TypeScript
//! module receives, and the `water` CLI turns that declaration into the
//! `.d.ts` the module is typed against. The declaration reaches the CLI
//! through the compiled artifact, never through the source text — a schema is
//! a `const` built from the field types themselves, so aliases, generic
//! parameters and `cfg`s are already resolved by the time anything reads it.
//!
//! ```
//! use waterui_ts_schema::{TsProps, TsType, TypeSchema};
//!
//! #[derive(TsType)]
//! struct Badge {
//!     label: String,
//!     count: u32,
//! }
//!
//! #[derive(TsProps)]
//! struct PromoProps {
//!     badge: Badge,
//!     dismissed: bool,
//! }
//!
//! // The schema is a constant, so it can be matched in a `const` context.
//! const _: () = assert!(matches!(PromoProps::SCHEMA, TypeSchema::Struct(_)));
//! assert_eq!(PromoProps::SCHEMA.to_string(), "PromoProps");
//!
//! // And so is the hash a downloaded bundle's manifest is checked against.
//! assert_eq!(
//!     PromoProps::CONTRACT_HASH,
//!     waterui_ts_schema::contract_hash(PromoProps::ENCODED)
//! );
//! ```
//!
//! # The artifact channel
//!
//! `#[derive(TsProps)]` emits a `#[cfg(debug_assertions)] #[used] static`
//! named `waterui_meta_tsprops_<Type>` holding the encoded schema followed by
//! one NUL. That is the canonical proc-macro to CLI channel: `#[used]` keeps
//! the item in the object file and the rlib, so the CLI enumerates it by name
//! from a dev-profile host build and reads its bytes. Because the whole
//! encoding happens during const evaluation, the static is data — the macro
//! runs no tool and reads no file.
//!
//! The debug gate is what keeps a shipped binary free of it: `#[used]` emits
//! `no_dead_strip` on Mach-O, so a release artifact would carry the payload
//! all the way into the application.
//!
//! # The component catalog
//!
//! The same format carries a second payload: [`CatalogSchema`], the runtime's
//! component table — which components JSX may name, what each one's
//! attributes are, and which modifier attributes exist. It is built from the
//! same [`TypeSchema`] constants, encoded by [`encode_catalog`] during const
//! evaluation, and read back by [`decode_catalog`].
//!
//! A third payload records a *mount point*: one `tsx!` call site, naming the
//! module it mounts and the props contract it is typed against. See
//! [`encode_mount`] and [`decode_mount`]. A fourth carries one half of the
//! *runtime fingerprint* — the JavaScript library's hash or the component
//! catalog's — see [`encode_runtime_half`], [`decode_runtime_half`] and
//! [`RuntimeFingerprint`]. The four kinds are told apart by the byte after
//! the version, so no decoder can read another kind as a malformed payload of
//! its own.
//!
//! # The bundle manifest
//!
//! The same crate defines what a built bundle says about itself and what its
//! signature covers: [`BundleManifest`] and [`SignedManifest`]. The `water`
//! CLI writes and signs them; the runtime's bundle loader verifies them. They
//! live here so both sides serialize the signed bytes through one type.
//!
//! # Type mapping
//!
//! | Rust | TypeScript | Node |
//! | --- | --- | --- |
//! | `Binding<T>` | `Signal<T>` | [`TypeSchema::Signal`] |
//! | `Computed<T>` | `Accessor<T>` | [`TypeSchema::Accessor`] |
//! | `AnyView` | `View` | [`TypeSchema::View`] |
//! | a JavaScript view builder | `() => JSX.Element` | [`TypeSchema::ViewBuilder`] |
//! | `Box`/`Rc<dyn Fn(A)>`, `fn(A)` — at most 8 arguments | `(arg0: A) => void` | [`TypeSchema::Callback`] |
//! | `#[derive(TsType)]` struct | object type | [`TypeSchema::Struct`] |
//! | `#[derive(TsType)]` enum | string union or tagged object | [`TypeSchema::Enum`] |
//! | `Option<T>` | `T \| null` | [`TypeSchema::Option`] |
//! | `Vec<T>`, `&'static [T]` | `T[]` | [`TypeSchema::List`] |
//! | `[T; N]` | `[T, …]`, a tuple of N | [`TypeSchema::Array`] |
//! | `BTreeMap<K, V>`, `HashMap<K, V>` | `Record<K, V>` | [`TypeSchema::Map`] |
//! | `String`, `Str`, `&'static str` | `string` | [`TypeSchema::String`] |
//! | `f32`, `f64`, integers to 32 bits | `number` | [`TypeSchema::Number`] |
//! | `i64`, `u64`, `isize`, `usize` | `bigint` | [`TypeSchema::Number`] |
//! | `bool` | `boolean` | [`TypeSchema::Bool`] |
//! | `()` | `void` | [`TypeSchema::Unit`] |
//!
//! There is no implicit fallback: a field whose type implements no [`TsType`]
//! is a compile error.

// The derives expand to `::waterui_ts_schema::…` so one expansion works inside
// this crate, in its tests and doctests, and in a dependent crate alike.
extern crate self as waterui_ts_schema;

mod catalog;
mod decode;
mod encode;
pub mod format;
mod impls;
mod manifest;
mod mount;
pub mod owned;
mod runtime;
mod tree;

#[cfg(test)]
mod tests;

pub use catalog::{
    Catalog, CatalogSchema, ChildrenSlot, Component, ComponentSchema, Modifier, ModifierSchema,
    attributes_of, catalog_encoded_len, decode_catalog, encode_catalog,
};
pub use decode::{DecodeError, decode};
pub use encode::{HASH_BASIS, contract_hash, encode, encoded_len, hash_extend, payload};
pub use format::{FORMAT_VERSION, MAX_ARRAY_LEN, MAX_DEPTH};
pub use manifest::{
    BundleFile, BundleManifest, ContractHash, HexBytes, HexError, Sha256Digest, SignatureBytes,
    SignedManifest,
};
pub use mount::{MountPoint, decode_mount, encode_mount, mount_encoded_len, struct_name};
pub use runtime::{
    FingerprintParseError, RuntimeFingerprint, RuntimeHalf, RuntimePart, decode_runtime_half,
    encode_runtime_half, runtime_half_encoded_len,
};
pub use tree::{
    EnumRepresentation, EnumSchema, FieldSchema, NumberKind, StructSchema, TypeSchema,
    VariantPayload, VariantSchema,
};

/// Derives the TypeScript projection of a struct or enum.
///
/// A struct becomes an object type and so needs named fields. An enum whose
/// variants are all unit variants becomes a union of string literals; an enum
/// carrying data becomes a tagged object, and the schema records which, so the
/// runtime converter follows the contract rather than a `serde` attribute.
///
/// Use this for the types nested inside a props struct. The root props struct
/// derives [`macro@TsProps`] instead, which also emits the artifact metadata.
///
/// Where the TypeScript runtime is reachable — a crate that depends on
/// `waterui` or `waterui-ts` — the derive also emits the `IntoJs` and `FromJs`
/// conversions that carry a value of the type across the seam this schema
/// describes, because a nested data type travels both ways: out as a props
/// field, back in as a callback argument. A type carrying a value that only
/// travels outwards — a callback, which the bridge registers rather than
/// reads — declares `#[ts(one_way)]` and gets `IntoJs` alone.
///
/// A field whose type has no [`TsType`] projection is a compile error naming
/// the field and its type — there is no implicit fallback:
///
/// ```compile_fail
/// use waterui_ts_schema::TsType;
///
/// struct Opaque;
///
/// #[derive(TsType)]
/// struct Card {
///     opaque: Opaque,
/// }
/// ```
#[cfg(feature = "derive")]
pub use waterui_macros::TsType;

/// Derives the props contract of one mounted TypeScript module.
///
/// Emits everything [`macro@TsType`] does, plus [`TsProps::ENCODED`],
/// [`TsProps::CONTRACT_HASH`] and the `waterui_meta_tsprops_<Type>` artifact
/// static the `water` CLI reads back.
///
/// Where the TypeScript runtime is reachable the derive also emits `IntoJs`,
/// and only `IntoJs`: props are handed to a module, and nothing reads a props
/// struct back out of JavaScript — requiring it to be readable would rule out
/// the callbacks and views props exist to carry.
///
/// Props are the object a module receives, so the root type is a struct with
/// named fields:
///
/// ```compile_fail
/// use waterui_ts_schema::TsProps;
///
/// #[derive(TsProps)]
/// struct PromoProps(String);
/// ```
#[cfg(feature = "derive")]
pub use waterui_macros::TsProps;

/// A type with a TypeScript projection.
///
/// The schema is an associated constant, so a field composes by naming its own
/// type's constant and the compiler resolves aliases and generic parameters
/// before anything is encoded. Implement it with `#[derive(TsType)]` for
/// application structs and enums; the mapped built-in types implement it here.
///
/// Callbacks — `Box<dyn Fn(..)>` in every `Send`/`Sync` flavour, bare
/// `Rc<dyn Fn(..)>`, and `fn(..)` pointers — project with at most eight
/// arguments; a wider signature has no schema and fails the bound.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no TypeScript projection and cannot cross the props seam",
    label = "no `TsType` schema for `{Self}`",
    note = "props fields must be mapped types: `Binding<T>`, `Computed<T>`, `AnyView`, \
            `Box`/`Rc<dyn Fn(..)>` or a `fn(..)` pointer of at most 8 arguments, \
            `Option`, `Vec`, `BTreeMap`/`HashMap`, a string, a number, `bool`, or a \
            struct or enum deriving `TsType`",
    note = "there is no implicit fallback: add `#[derive(TsType)]` to `{Self}`, or change \
            the field's type"
)]
pub trait TsType {
    /// This type's projection.
    const SCHEMA: TypeSchema;
}

/// A type that may key a [`TypeSchema::Map`].
///
/// TypeScript object keys are strings, so a map only crosses the seam when its
/// key type projects to `string`. Making that a bound rather than a check
/// means a map keyed by anything else never reaches the encoder.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot key a TypeScript object",
    label = "`{Self}` does not project to `string`",
    note = "a map crossing the props seam must be keyed by `String`, `Str` or `&'static str`"
)]
pub trait TsMapKey: TsType {}

/// A root props struct: the contract one mounted TypeScript module is compiled
/// against.
///
/// Derived by `#[derive(TsProps)]`, which also emits the artifact static the
/// CLI reads.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a TypeScript props contract and cannot be mounted",
    label = "`{Self}` does not derive `TsProps`",
    note = "a mounted TypeScript module is typed against its props: add \
            `#[derive(TsProps)]` to `{Self}`, which is what gives it the contract hash a \
            bundle is checked against",
    note = "a module that takes no props mounts against `waterui::ts::NoProps`, which \
            `tsx!(\"./promo.tsx\")` with no props argument uses"
)]
pub trait TsProps: TsType {
    /// The encoded [`TsType::SCHEMA`], without the NUL terminator the artifact
    /// static appends — the exact bytes the CLI recovers and [`decode`] reads.
    const ENCODED: &'static [u8];

    /// [`contract_hash`] of [`Self::ENCODED`].
    ///
    /// A downloaded bundle's manifest carries the hash each module was built
    /// against; a bundle whose hash differs from the binary's is rejected
    /// before it is cached.
    const CONTRACT_HASH: u64;
}
