# waterui-str

A memory-efficient, reference-counted string type optimized for both static and owned strings.

## Overview

`waterui-str` provides `Str`, a hybrid string type that automatically chooses between static string references and reference-counted owned strings. This design eliminates unnecessary allocations for static strings while enabling efficient cloning for owned strings through reference counting.

The crate is designed for `no_std` environments (with `alloc`), making it suitable for embedded systems and WebAssembly targets. It integrates seamlessly with `WaterUI`'s reactive system through the `nami-core` integration.

Key features:
- **Zero-cost static strings**: Static string literals stored as pointers without allocation
- **Reference-counted owned strings**: Efficient cloning through internal reference counting
- **Transparent API**: Derefs to `&str`, works with all standard string operations
- **Reactive integration**: Compatible with `WaterUI`'s `nami` reactive primitives via `impl_constant!`
- **Optional serde support**: Serialize and deserialize with the `serde` feature

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-str = "0.2"
```

With serde support:

```toml
[dependencies]
waterui-str = { version = "0.2", features = ["serde"] }
```

## Quick Start

```rust
use waterui_str::Str;

// Static strings - no allocation
let static_str = Str::from("hello");

// Owned strings - reference counted
let owned = Str::from(String::from("world"));

// Cheap cloning
let clone = owned.clone(); // Just increments ref count

// Transparent string operations
assert_eq!(static_str.len(), 5);
assert!(static_str.starts_with("hel"));

// Concatenation
let combined = static_str + " " + &owned;
assert_eq!(combined, "hello world");
```

## Core Concepts

### Internal Representation

`Str` uses a clever tagged pointer representation:

- **Positive length**: Points to static string data (`&'static str`)
- **Negative length**: Points to `Shared` (reference-counted `String`)

This allows zero-overhead discrimination between static and owned strings at runtime.

### Reference Counting

Owned strings use internal reference counting via `Shared`:
- Clone operations increment the reference count
- Drop operations decrement the count and free memory when reaching zero
- Reference counts are intentionally not exposed in the public API

### Memory Optimization

Empty strings always use a static empty string reference, regardless of how they're created:

```rust
use waterui_str::Str;

let empty1 = Str::new();
let empty2 = Str::from("");
let empty3 = Str::from(String::new());

// All three use the same static "" reference
```

## Examples

### Creating Strings

```rust
use waterui_str::Str;

// From static string literal
let s1 = Str::from("hello");

// From owned String
let s2 = Str::from(String::from("hello"));

// From UTF-8 bytes
let bytes = vec![104, 101, 108, 108, 111]; // "hello"
let s3 = Str::from_utf8(bytes).unwrap();

// Empty string
let s4 = Str::new();
```

### String Manipulation

```rust
use waterui_str::Str;

let mut s = Str::from("hello");
s.append(" world");
assert_eq!(s, "hello world");

// Concatenation with +
let s1 = Str::from("foo");
let s2 = s1 + "bar";
assert_eq!(s2, "foobar");

// AddAssign
let mut s3 = Str::from("hello");
s3 += " world";
assert_eq!(s3, "hello world");
```

### Iteration and Collection

```rust
use waterui_str::Str;

// Collect from iterator
let words = vec!["hello", " ", "world"];
let s: Str = words.into_iter().collect();
assert_eq!(s, "hello world");

// Extend
let mut s = Str::from("hello");
s.extend(vec![" ", "world"]);
assert_eq!(s, "hello world");
```

### Conversion to String

```rust
use waterui_str::Str;

let s1 = Str::from(String::from("owned"));
let s2 = s1.clone();

// Convert to String - takes ownership if ref count is 1
let string1 = s1.into_string(); // Copies because s2 still exists
assert_eq!(string1, "owned");

// s2 is now the only reference
let string2 = s2.into_string(); // No copy, takes ownership
assert_eq!(string2, "owned");
```

## API Overview

### Construction
- `Str::new()` - Create empty string
- `Str::from_static(&'static str)` - Create from static string literal
- `Str::from_utf8(Vec<u8>)` - Create from UTF-8 bytes with validation
- `unsafe Str::from_utf8_unchecked(Vec<u8>)` - Create from UTF-8 bytes without validation

### Inspection
- `as_str(&self) -> &str` - Get string slice
- `len(&self) -> usize` - Get byte length
- `is_empty(&self) -> bool` - Check if empty

### Modification
- `append(&mut self, &str)` - Append string
- `into_string(self) -> String` - Convert to owned String

### Traits Implemented
- `Deref<Target = str>` - Transparent access to string methods
- `Clone` - Efficient reference-counted cloning
- `Default` - Empty string
- `Display`, `Debug` - Formatting
- `Hash`, `Eq`, `Ord` - Collections and comparisons
- `AsRef<str>`, `AsRef<[u8]>`, `Borrow<str>` - Conversions
- `FromStr`, `FromIterator` - Parsing and collection
- `Add`, `AddAssign` - Concatenation
- `Extend` - Extension from iterators
- `Index<I>` - Slice indexing

### Standard Library Integration (when `std` is available)
- `AsRef<OsStr>`, `AsRef<Path>` - Filesystem operations
- `TryFrom<OsString>` - OS string conversion
- `ToSocketAddrs` - Network address resolution

## Design Rationale

### Why Not `Cow<'static, str>`?

While `Cow<'static, str>` provides similar functionality, `Str` offers:
- **Better clone performance**: Reference counting vs. full string copy for `Cow::Owned`
- **Smaller size**: Single pointer + length vs. discriminant + pointer + length
- **Specialized API**: Methods like `append()` optimized for the use case

### Why Hide Reference Counts?

The internal reference count is deliberately not exposed in the public API. This:
- Prevents code from relying on reference count values
- Allows future optimization changes without breaking the API
- Encourages treating `Str` as a simple value type

### Memory Safety

The crate includes extensive memory safety tests designed for Miri (Rust's undefined behavior detector), covering:
- Clone/drop cycle patterns
- Interleaved operations
- Reference counting edge cases
- Pointer stability guarantees
- Large string handling
- Concurrent-like access patterns (single-threaded stress tests)

## Performance Characteristics

- **Static strings**: Zero allocation, zero cost to clone
- **Owned strings**: Single allocation, O(1) clone (ref count increment)
- **Deref operations**: Zero cost - compiles to a pointer dereference
- **`into_string()` with unique ownership**: Zero copy, takes ownership
- **`into_string()` with shared ownership**: Single allocation and copy

## License

Licensed under the same terms as the `WaterUI` project.
