# waterui-url

Fast, zero-allocation URL parsing for WaterUI with compile-time validation and reactive fetching.

## Overview

`waterui-url` provides a unified `Url` type that handles web URLs, local file paths, data URLs, and blob URLs with a consistent API. The library is designed for performance and safety, offering compile-time URL creation with validation, zero-allocation parsing using byte offsets, and seamless integration with WaterUI's reactive primitives.

Key features:

- **Compile-time URL creation**: Create and validate URLs at compile time using `const fn`
- **Zero-allocation parsing**: Parses URLs once, stores byte offsets to components, no intermediate allocations
- **Unified API**: Single `Url` type for web URLs, local paths, data URLs, and blob URLs
- **O(1) component access**: Scheme, host, port, path, query, and fragment are extracted during parsing
- **no_std compatible**: Works without the standard library (with optional `std` feature for `Path` support)
- **Reactive integration**: Built-in support for reactive URL fetching via `nami` signals

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
waterui-url = "0.1.0"
```

For file path support (requires `std`):

```toml
[dependencies]
waterui-url = { version = "0.1.0", features = ["std"] }
```

## Quick Start

```rust
use waterui_url::Url;

// Compile-time URL creation with validation
const API_ENDPOINT: Url = Url::new("https://api.example.com/v1/users");
const LOCAL_ASSET: Url = Url::new("/assets/logo.png");

fn main() {
    // Runtime parsing
    let url: Url = "https://example.com/api?key=value".parse().unwrap();

    println!("Host: {:?}", url.host());        // Some("example.com")
    println!("Path: {}", url.path());          // "/api"
    println!("Query: {:?}", url.query());      // Some("key=value")
}
```

## Core Concepts

### Url Type

The `Url` struct is a unified type that automatically detects and parses different URL kinds:

- **Web URLs**: `http://`, `https://`, `ftp://`, `ws://`, `wss://`, etc.
- **Local file paths**: Absolute (`/home/user/file.txt`) or relative (`./images/photo.jpg`)
- **Data URLs**: `data:text/plain;base64,SGVsbG8=`
- **Blob URLs**: `blob:https://example.com/uuid`

The parser runs once during construction and stores component offsets as `u16` byte ranges, making subsequent access to URL parts O(1) with no additional parsing.

### Compile-Time Validation

URLs created with `Url::new()` in const contexts are validated at compile time. Malformed URLs cause compilation errors:

```rust
// This compiles fine
const VALID: Url = Url::new("https://example.com");

// This fails at compile time: "Web URL must have a host"
const INVALID: Url = Url::new("https://");
```

### Reactive Fetching

The `fetch()` method returns a reactive signal that can be watched for changes, integrating with WaterUI's reactive update system:

```rust
use waterui_url::Url;

let url = Url::parse("https://api.example.com/data.json").unwrap();
let fetched = url.fetch();
// Use with WaterUI's reactive primitives
```

## Examples

### Parsing Web URLs

```rust
use waterui_url::Url;

let url = Url::parse("https://user:pass@example.com:8080/api/v1?id=123#section").unwrap();

assert_eq!(url.scheme(), Some("https"));
assert_eq!(url.host(), Some("example.com"));
assert_eq!(url.port(), Some(8080));
assert_eq!(url.path(), "/api/v1");
assert_eq!(url.query(), Some("id=123"));
assert_eq!(url.fragment(), Some("section"));
assert_eq!(url.authority(), Some("user:pass@example.com:8080"));
```

### Working with Local Paths

```rust
use waterui_url::Url;

let absolute = Url::new("/home/user/documents/file.pdf");
assert!(absolute.is_local());
assert!(absolute.is_absolute());
assert_eq!(absolute.extension(), Some("pdf"));
assert_eq!(absolute.filename(), Some("file.pdf"));

let relative = Url::new("./images/photo.jpg");
assert!(relative.is_relative());
assert_eq!(relative.extension(), Some("jpg"));
```

### Creating Data URLs

```rust
use waterui_url::Url;

let data_url = Url::from_data("image/png", b"PNG binary data here...");
assert!(data_url.is_data());
assert_eq!(data_url.scheme(), Some("data"));
// URL is automatically base64-encoded
println!("{}", data_url);  // data:image/png;base64,UE5HIGJpbmFyeSBkYXRhIGhlcmUuLi4=
```

### Joining URLs

```rust
use waterui_url::Url;

let base = Url::new("https://cdn.example.com/assets/");
let joined = base.join("images/logo.png");
assert_eq!(joined.as_str(), "https://cdn.example.com/assets/images/logo.png");

// Handles file paths too
let base_file = Url::new("https://example.com/path/file.html");
let joined_file = base_file.join("other.html");
assert_eq!(joined_file.as_str(), "https://example.com/path/other.html");
```

## API Overview

### Construction

- `Url::new(url: &'static str) -> Url` - Create URL at compile time (const fn)
- `Url::parse(url: impl AsRef<str>) -> Option<Url>` - Parse web URL at runtime
- `Url::from_file_path(path: impl AsRef<Path>) -> Url` - Create from file path (requires `std` feature)
- `Url::from_file_path_str(path: impl Into<Str>) -> Url` - Create from path string
- `Url::from_data(mime_type: &str, data: &[u8]) -> Url` - Create base64-encoded data URL
- `FromStr` trait - Parse any URL type from string

### Type Checking

- `is_web() -> bool` - Check if URL is a web URL (http/https/ftp/ws/wss)
- `is_local() -> bool` - Check if URL is a local file path
- `is_data() -> bool` - Check if URL is a data URL
- `is_blob() -> bool` - Check if URL is a blob URL
- `is_absolute() -> bool` - Check if URL or path is absolute
- `is_relative() -> bool` - Check if URL or path is relative

### Component Access

All component accessors are O(1) operations using pre-parsed byte offsets:

- `scheme() -> Option<&str>` - URL scheme (e.g., "https", "file", "data")
- `host() -> Option<&str>` - Host portion for web URLs
- `port() -> Option<u16>` - Port number for web URLs (parsed as integer)
- `path() -> &str` - Path component
- `query() -> Option<&str>` - Query string without '?'
- `fragment() -> Option<&str>` - Fragment without '#'
- `authority() -> Option<&str>` - Full authority section (user:pass@host:port)
- `extension() -> Option<&str>` - File extension if present
- `filename() -> Option<&str>` - Filename from path

### Manipulation

- `join(&self, path: &str) -> Url` - Join URL with relative path
- `fetch(&self) -> Fetched` - Get reactive signal for URL content
- `to_file_path(&self) -> Option<PathBuf>` - Convert to file path (requires `std` feature)

### Conversion

- `as_str(&self) -> &str` - Get URL as string slice
- `into_string(self) -> String` - Convert to owned String
- `inner(&self) -> Str` - Get the underlying `Str` value
- `AsRef<str>` - Automatic conversion to `&str`
- `Display` - Format URL as string

## Features

The crate supports the following Cargo features:

- `std` (optional): Enables `std::path::Path` integration for `from_file_path()` and `to_file_path()` methods. Without this feature, the crate is `no_std` compatible.

## Implementation Details

### Zero-Allocation Architecture

The `Url` type stores the original URL string once and references its components using `Span` structs containing `u16` start/end offsets. This design:

- Parses the URL exactly once during construction
- Stores component offsets, not copies of component strings
- Returns string slices that reference the original URL
- Uses only 2 bytes per component (start + end offset)
- Supports URLs up to 65,535 bytes (reasonable for most use cases)

### Const-Compatible Parser

The entire parsing implementation uses const-compatible operations (byte-level comparisons, loops with manual indexing) to enable compile-time URL creation and validation. The parser:

- Validates web URL structure (requires scheme and host)
- Checks port numbers are valid (digits only, max 5 digits)
- Handles IPv6 addresses in brackets `[::1]`
- Parses data URLs with MIME types and base64 encoding
- Detects Windows vs Unix path conventions

### Error Handling

The `parse()` function returns `Result<Url, ParseError>` for runtime parsing. The `new()` const fn panics on invalid URLs, enabling compile-time validation in const contexts.

## Platform Support

This crate is `no_std` compatible by default. Enable the `std` feature for:
- `Path`/`PathBuf` integration via `from_file_path()` and `to_file_path()`
- `std::error::Error` trait implementation for `ParseError`

Windows, macOS, and Linux are all supported. The parser correctly handles platform-specific path conventions (backslashes, drive letters on Windows).
