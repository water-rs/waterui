# waterui-assets

Asset management primitives for WaterUI applications.

`waterui-assets` provides typed asset values for small data files, large
memory-mapped files, remote downloads, and asset kind classification. The crate
is the runtime side of WaterUI's asset system; build-time discovery and generated
asset modules are handled by companion tooling crates.

## Features

- `Data` for small binary resources loaded into memory.
- `LargeFile` for memory-mapped large files such as models or media blobs.
- `AssetKind` for classifying assets by extension.
- Remote download helpers with atomic writes for cache population.

## Usage

Most applications use this crate through generated `asset!` bindings. Direct
users can construct `Data` or `LargeFile` values from local files or remote
URLs.
