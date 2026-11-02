# Bundles over the air

How an application built with `waterui-ts` gets its TypeScript view bundle
at launch, how a newer bundle reaches it, and what has to be true of that
bundle before it is trusted. This is the contract between three parties:
the `water` CLI, which builds, signs and publishes bundles and generates the
leaf crate; the leaf crate, which embeds the baseline, the requirement and
the public key and drives the loader; and the runtime in this crate, which
verifies, caches, selects and evaluates.

## Terms

| Term | What it is |
| --- | --- |
| **bundle** | One classic script: the JavaScript library plus every `.tsx` module of the application, ending in `installRuntimeGlobal(modules, contracts)`. One application, one bundle. |
| **baseline** | The bundle built into the binary, with its manifest. The floor: a launch can always fall back to it, and it is never marked bad. |
| **requirement** | The binary's side: its runtime fingerprint and, for every module it mounts, the module id and the props contract hash it was compiled with. A `const` in the leaf crate. |
| **bundle manifest** | The bundle's side: version, runtime fingerprint, bundle file digest, per-module contract hashes, optional translations. JSON. |
| **runtime fingerprint** | Which runtime a bundle was built for: the schema format version, the JavaScript library's hash and the component catalog's hash. |
| **store** | The on-disk cache of verified downloaded bundles and the record of which versions failed. |

## The runtime fingerprint

A bundle calls into the JavaScript library (`src/js/*.js`) by name and
names components out of the catalog the facade publishes. Both change with
the framework, so a bundle is only valid for the runtime it was compiled
against, and the fingerprint is how that is checked.

The two halves are computed by the compiler in the crate that owns each:

| Half | Constant | Artifact static | Crate |
| --- | --- | --- | --- |
| library | `waterui_ts::LIBRARY_HASH` | `waterui_meta_ts_runtime_library` | `waterui-ts` |
| catalog | `waterui::ts::catalog::CATALOG_HASH` | `waterui_meta_ts_runtime_catalog` | `waterui-internal` (the facade) |

The library hash is one FNV-1a 64 chain over every `.js` file of the
library in name order, `name`, NUL, contents, NUL per file
(`waterui_ts_schema::hash_extend` from `HASH_BASIS`); only the sixty-four
bits reach the binary. The catalog hash is `contract_hash` over the encoded
catalog payload, the same bytes `waterui_meta_ts_catalog` carries.

Each static is a `#[cfg(debug_assertions)] #[used]` payload in the schema
crate's format, kind byte `0x22` (`RUNTIME_HALF`), followed by a part byte
(`1` library, `2` catalog) and the hash as the format's base-127 varint.
`encode_runtime_half` writes one, `decode_runtime_half` reads it back.

The CLI never recomputes a hash from source. It reads the two statics out of
the two rlibs of the build it is packaging — the same read it does for
`waterui_meta_tsx_*` and `waterui_meta_ts_catalog` — decodes them, and
combines them:

```rust
let library = decode_runtime_half(&library_static)?;   // part == Library
let catalog = decode_runtime_half(&catalog_static)?;   // part == Catalog
let fingerprint = RuntimeFingerprint::new(library.hash, catalog.hash);
manifest.runtime = fingerprint;                         // serializes as text
```

The text form is `<format>-<library>-<catalog>` with each hash as sixteen
lowercase hexadecimal digits, for example
`2-9f86d081884c7d65-9b71d224bd62f378`. The format version is the schema
crate's `FORMAT_VERSION`, which is also the first byte of every payload the
CLI decodes, so a CLI on another format generation cannot produce a
fingerprint for this one.

The binary computes the same value at compile time:
`waterui::ts::RUNTIME_FINGERPRINT` is
`RuntimeFingerprint::new(LIBRARY_HASH, CATALOG_HASH)`. The leaf crate names
that constant in its requirement, so the loader compares the manifest
against a value derived from the same two constants the CLI read out of the
artifacts. `tests/fingerprint.rs` holds the two to each other by reading the
statics back out of the test executable.

## The requirement manifest

The leaf crate the CLI generates instantiates it as a `const`:

```rust
use waterui::ts::{RequiredModule, Requirement, RUNTIME_FINGERPRINT};

const REQUIREMENT: Requirement = Requirement::new(
    RUNTIME_FINGERPRINT,
    &[
        RequiredModule::new("src/views/promo.tsx", 0x0123_4567_89ab_cdef),
        RequiredModule::new("src/views/about.tsx", 0xfedc_ba98_7654_3210),
    ],
);
```

Each `RequiredModule` is one `waterui_meta_tsx_*` mount point: the module id
(the `.tsx` path relative to the crate's manifest directory, forward-slashed,
extension kept) and `TsProps::CONTRACT_HASH` of the props type it is mounted
with, written as the literal the CLI decoded. A module mounted with no props
is required at `NoProps::CONTRACT_HASH`. `waterui_ts_schema::MountPoint` is
not reused here because it is the owned, `String`-carrying record a decoder
produces; the requirement is a `const` the leaf crate writes, and it needs
`&'static str`.

## The bundle manifest

Written by the CLI beside every bundle it builds. The types are in
`waterui-ts-schema` (`BundleManifest`, `SignedManifest`), so the CLI and the
runtime serialize the signed bytes through one type.

```json
{
  "manifest": {
    "version": 3,
    "runtime": "2-9f86d081884c7d65-9b71d224bd62f378",
    "bundle": {
      "url": "bundle-3.js",
      "size": 48213,
      "sha256": "<64 lowercase hex digits>"
    },
    "modules": {
      "src/views/about.tsx": "fedcba9876543210",
      "src/views/promo.tsx": "0123456789abcdef"
    },
    "translations": {
      "en": "greeting = \"Hello\"\n",
      "zh-Hans": "greeting = \"你好\"\n"
    }
  },
  "signature": "<128 lowercase hex digits>"
}
```

| Member | Meaning |
| --- | --- |
| `version` | A `u64`. Larger is newer. The loader prefers the newest verified bundle, and a published bundle is only ever preferred to the baseline when its version is greater than the baseline's. The CLI assigns it; the baseline's version is the version the embedded bundle was built as. |
| `runtime` | The runtime fingerprint, as text. |
| `bundle.url` | Where the bundle file is, resolved against the manifest's own URL (`Url::join`), so a manifest and its bundle can be uploaded together anywhere. For the baseline it is the file name the CLI wrote beside it. |
| `bundle.size` | The bundle file's length in bytes, a `u64`. The bound the client reads the download under: a response that declares or delivers more is refused without being buffered past it. Signed, so the publisher and not the server decides how much memory a fetch may take. |
| `bundle.sha256` | SHA-256 of the bundle file's bytes. |
| `modules` | Every module the bundle carries, keyed by module id, valued by the contract hash it was built against as sixteen lowercase hexadecimal digits — the same spelling as `installRuntimeGlobal`'s `contracts`. |
| `translations` | Locale tag to the TOML text of that locale's translation file, exactly the document `TranslationCatalog::add_toml` takes. Omitted when the bundle carries none. |
| `signature` | ed25519 over the signed bytes below. |

The baseline embeds the inner `manifest` object alone, unsigned: the
binary's own code signature covers it. `deny_unknown_fields` is on, so a
member this version does not know is a parse error rather than silently
dropped from the signed bytes.

### The signed bytes

`BundleManifest::signed_bytes()`: the compact JSON serialization of the
inner `manifest` object as `serde_json` writes this type. Spelled out so
another implementation can reproduce it byte for byte:

- UTF-8, no whitespace anywhere;
- the members `version`, `runtime`, `bundle`, `modules` and — only when
  non-empty — `translations`, in that order; `bundle`'s members `url`,
  `size` then `sha256`;
- the entries of `modules` and `translations` sorted by key as byte strings
  (`BTreeMap` order);
- `version` and `size` in plain decimal; every hash as lowercase
  hexadecimal of its fixed width;
- strings quoted with `\"`, `\\`, `\n`, `\r`, `\t`, `\b` and `\f` as
  two-character escapes, every other control character (U+0000–U+001F) as
  `\u00XX` with lowercase hex digits, and nothing else escaped — non-ASCII
  text is written as UTF-8.

The bundle file's bytes are covered through `bundle.sha256`. That is why a
manifest can be verified before the bundle is downloaded, and why the fetch
downloads nothing for a manifest that fails.

For the manifest above, with the digest written out as `ab` sixty-four
times, the signed bytes are:

```text
{"version":3,"runtime":"2-9f86d081884c7d65-9b71d224bd62f378","bundle":{"url":"bundle-3.js","size":48213,"sha256":"abababababababababababababababababababababababababababababababab"},"modules":{"src/views/about.tsx":"fedcba9876543210","src/views/promo.tsx":"0123456789abcdef"},"translations":{"en":"greeting = \"Hello\"\n","zh-Hans":"greeting = \"你好\"\n"}}
```

The verifier never trusts the bytes it received to be canonical: it parses
the document, re-serializes the parsed manifest through the same type, and
checks the signature over that. Whitespace, member order and escaping in the
served file do not matter; only the content does.

Signing, for `water ota publish` (`ed25519-dalek`):

```rust
let manifest: BundleManifest = /* built from the bundle step */;
let signature = signing_key.sign(&manifest.signed_bytes());
let published = SignedManifest {
    manifest,
    signature: SignatureBytes::new(signature.to_bytes()),
};
std::fs::write("manifest.json", published.to_json())?;
```

Verification uses `VerifyingKey::verify_strict`, which refuses the
non-canonical signature encodings a lenient verifier accepts: there is
exactly one signature for one document.

## Verification

In this order, each step a typed `Rejection` naming what it found and logged
through `tracing` by whoever decides what to do about it:

1. **Signature** — `verify_strict` under the embedded key over the signed
   bytes. Downloaded and cached bundles only; the baseline has none.
2. **Runtime fingerprint** — `manifest.runtime == requirement.fingerprint`.
3. **Every mounted module** — each `RequiredModule` is in `manifest.modules`
   with an equal contract hash. A missing module and a mismatched hash are
   distinct rejections, and the mismatch names both hashes.
4. **Translations** — every key parses as a `Locale` and every document as
   a translation file. Checked here, before the catalog is built, because
   `TranslationCatalog::add_toml` treats an invalid locale as a programming
   error and a downloaded document is an input.
5. **Size** — the bundle file is no longer than `bundle.size`. For a
   download, a `Content-Length` above it refuses the response before its
   body is read, and a body that runs past it is refused at the first chunk
   that crosses the bound (`FetchError::BodySize`, naming the bound, the
   declared length and the bytes read); for a cached file, its length on
   disk is checked and the read itself stops at the bound, so a file that
   grows under the reader is refused too. The bytes buffered never exceed
   the bound. The manifest, which nothing signed bounds, is read under the
   protocol's `MANIFEST_SIZE_LIMIT` of one mebibyte the same way.
6. **Digest** — SHA-256 of the bundle file's bytes equals `bundle.sha256`,
   and the bytes are UTF-8.

Nothing is written to the store before every step has passed. A fetch that
fails at step 1–4 never requests the bundle file.

## The store

Under the application's cache directory by default
(`BundleStore::in_cache_dir(namespace)`, `waterkit-fs`'s `cache_dir()`
joined with the application's bundle identifier and `waterui-ts`); the
general constructor `BundleStore::new(root)` takes any directory, which is
how tests inject a temporary one.

```text
<root>/
  state.json              { "bad": [4], "booting": 5 }
  bundles/
    5/
      bundle.js
      manifest.json       the SignedManifest as downloaded
```

One directory per version, written whole: both files go into a staging
directory (`bundles/.<version>.staging`) and the directory is renamed into
place, so a version directory that exists is a complete one; a staging
directory a fetch died in is reaped at the next launch. `state.json` holds
the versions that failed and the version currently booting, and is written
the same way — to a sibling file, flushed, then renamed over the old one —
so a launch never finds half a state file. One it cannot read or parse
anyway is logged at warn, naming the file and the reason, and replaced by
the empty state: the store is a cache, and a bad mark that is lost is
re-learned the next time that version fails.

Why the cache directory: it is the one directory `waterkit-fs` reaches on
every platform that is app-private and regenerable. `documents_dir` is
user-visible on iOS and Android; `data_local_path` exists only on desktop.
A cache purge costs one re-download and nothing else, because the baseline
is the floor and every launch re-verifies what it finds.

## Selection at launch

`Loader::load(&ota)`, cold start only — there is no runtime hot swap:

1. A `booting` record left set means the previous launch died between
   evaluating that version and reporting that it booted — a `tsx!` mount
   that panicked, typically. The version is marked bad and removed.
2. The baseline is parsed and verified against the requirement. A baseline
   that does not match is a `LaunchError`, never a bundle to fall past: it
   is a build-pipeline bug.
3. Every cached version at or below the baseline's is stale and removed;
   bad-list entries at or below it are forgotten, which bounds the list.
4. The remaining versions, newest first, skipping the bad ones. Each is read
   and verified (signature, digest, requirement). One that fails is removed
   and not marked bad — it is either a bundle for a binary this no longer is
   or a cache that was altered, and marking it bad would poison that version
   for the binary it was published for. One that verifies is recorded as
   `booting` and evaluated in a fresh runtime. One that throws is marked
   bad, removed, and the walk continues in the same launch. One that
   evaluates is the launch.
5. Nothing usable: the baseline.

The store is a cache, and no failure of it is a failure of the launch. The
only hard errors `Loader::load` has are the baseline's own —
`LaunchError::BaselineManifest`, `BaselineRejected`, `BaselineFailed` — and
`LaunchError::Engine`; there is no store variant. Every store failure along
the walk is logged at warn and fallen past:

| Failure | What the launch does |
| --- | --- |
| `state.json` unreadable or unparsable | The empty state, written back over it. |
| `bundles/` cannot be listed | No cached candidates: the baseline. |
| A version cannot be read, or is not the version it is filed under | Removed if it can be, skipped for this launch either way. `bundles/7` as a regular file is this case: it is removed so 7 can be cached later. Not marked bad — nothing was evaluated. |
| A version cannot be removed | Skipped for this launch; tried again at the next. |
| The state cannot be saved | The mark it carried is re-learned when the version fails again. |
| The boot record cannot be written | That version is not evaluated: without the record a mount that panics would be retried at every launch. The walk continues. |
| `Launched::booted()` cannot clear the record | Logged; the next launch marks the version bad and re-downloads it. |

A stale `.<version>.staging` directory is removed before the walk starts.

`Loader::baseline_only()` is step 2 and 5 alone: no store is read, no file
is touched. It is what a build without an update source calls, and a build
without the `ota` feature has nothing else.

### The boot record

`Launched::booted()` clears it. The CLI-generated leaf crate makes this call
once the application's first frame has been presented — every `tsx!` mount in
the initial tree has run by then — and not before: a mount can still panic
until then, and the record is what turns that panic into a bad mark at the
next launch instead of a crash loop. For the baseline the call does nothing.
It returns nothing: a record that cannot be cleared is logged at warn, and
the application keeps running on the bundle that just booted.

## The fetch

`Ota::fetch(&requirement, baseline_version)`, or
`Launched::spawn_update(ota)` which spawns it on the local executor after
launch and reports through `tracing`. It never delays a frame and only ever
writes to the store.

1. `GET` the manifest URL; parse as `SignedManifest`; verify the signature.
2. `version <= baseline` → `Outcome::NotNewer`; in the bad list →
   `Outcome::KnownBad`; already cached → `Outcome::AlreadyCached`. No
   download in any of these.
3. Verify the requirement (fingerprint, modules, translations).
4. `GET` `bundle.url` resolved against the manifest URL, read under
   `bundle.size`; verify the digest.
5. Write both files to the store → `Outcome::Cached`.

A network failure is `FetchError::Network`, logged at debug: the
application is running on whatever it launched with. A rejection is logged at
warn with its reason. The client fetches the application's own view bundle
and nothing else — no other code, no other resource — which is what the
store policies this feature exists under permit.

## Translations

A bundle that carries `translations` installs them as the
`TranslationCatalog` of the environment the launch hands back, so every
`<Text>` in its modules — and every Rust `text("…")` rendered under that
environment — looks keys up in the bundle's catalog. It is the whole table,
not a delta: the CLI publishes the application's complete `locales/` set,
and a bundle's catalog shadows the one the application compiled in. That is
what lets an update ship new strings alongside new views; a bundle that
carries none leaves the application's catalog in place.

## What the leaf crate does

```rust
use waterui::ts::{Baseline, BundleStore, Components, Loader, Ota, Requirement, RequiredModule, RUNTIME_FINGERPRINT};

const REQUIREMENT: Requirement = Requirement::new(RUNTIME_FINGERPRINT, &[/* mounts */]);
const BASELINE: Baseline = Baseline::new(include_str!("bundle.js"), include_str!("manifest.json"));
const PUBLIC_KEY: [u8; 32] = [/* water ota keygen */];

// With an update source configured (feature `ts-ota`):
let ota = Ota::new(PUBLIC_KEY, MANIFEST_URL, BundleStore::in_cache_dir(BUNDLE_ID)?)?;
let launched = Loader::new(REQUIREMENT, BASELINE, Components, environment).load(&ota)?;
let environment = launched.environment().clone();   // the root renders under this
launched.spawn_update(ota);                          // after launch, never before the first frame
// ... once the first frame is presented:
launched.booted();

// Without one (feature `ts` alone): no store, no key, no URL, no client type.
let launched = Loader::new(REQUIREMENT, BASELINE, Components, environment).baseline_only()?;
```

## Features

| Feature | Links | Present without it |
| --- | --- | --- |
| `waterui-ts` (always) | `serde`, `serde_json`, `sha2` | `Requirement`, `RequiredModule`, `Baseline`, `Loader::baseline_only`, `Launched`, `Rejection`, `LaunchError`, the manifest types via `schema`, `LIBRARY_HASH` |
| `waterui-ts/ota` (`waterui/ts-ota`) | `ed25519-dalek`, `zenwave` (platform TLS on Apple, rustls elsewhere), `futures-lite` (the bundle body as a stream), `waterkit-fs`, `url`, `executor-core`, `blocking` | `Ota`, `BundleStore`, `Loader::load`, `Launched::spawn_update`, `FetchError`, `Outcome`, `StoreError` |

`cargo tree -p waterui-ts -e normal -i zenwave` answers "nothing to print"
without the feature: a baseline-only application links no HTTP client. With
it, the client type exists only as an `Ota`, which cannot be constructed
without a manifest URL: `Ota::new(public_key, manifest_url, store)` is its
one constructor, and `Loader::baseline_only` takes no `Ota`.
