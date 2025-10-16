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
