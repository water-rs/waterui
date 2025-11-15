# waterui-ffi

This crate provides the Foreign Function Interface (FFI) for WaterUI.

## C Header

The C header file `waterui.h` is used by downstream backends (like Swift and Java/Kotlin) to interact with the Rust core.

### Updating the Header

The `waterui.h` header is generated using `cbindgen` and is checked into version control. To update it after making changes to the FFI API, run the following command from the root of the `waterui` repository:

```bash
cargo run --bin generate_header --features cbindgen --manifest-path ffi/Cargo.toml
```

The CI will verify that the header is up-to-date.

## How applications use the FFI

Every WaterUI application is just a normal Rust crate (it has its own `Cargo.toml` and `src/lib.rs`). That crate:

1. Depends on `waterui` to define the application logic (`init()` returns an `Environment`, `main()` returns a `View`, etc.).
2. Depends on `waterui-ffi` for the `waterui_ffi::export!()` macro, which expands to the `#[no_mangle] extern "C"` entry points `waterui_init` and `waterui_main`.
3. Re-exports those entry points so any native shell (Android, Apple, web) can call straight into the Rust code.

The CLI scaffolds a `build-rust.sh` next to each project. When you run `water run android`/`water package android`, the CLI executes that script before Gradle so Cargo builds **the project crate** (not the helper crate) for every requested Android ABI. The built `.so` is copied into `android/app/src/main/jniLibs/` and loaded by the Android runtime, which expects the exported `waterui_init`/`waterui_main` symbols documented above. Apple backends follow the same pattern: Xcode runs the generated `build-rust.sh`, links the resulting static library, and calls the exported functions via `waterui.h`.

Because the project crate owns the exports, any change you make to `init()`/`main()` is automatically available to every platform shell—there is no need to edit `waterui-ffi` itself, and you should never build `waterui-ffi` as a standalone artifact.
