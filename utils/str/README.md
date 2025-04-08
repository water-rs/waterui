A zero-copy string type that can either hold a static reference or a ref-counted owned string.

The `Str` type provides a unified interface for both static string references and dynamically
allocated strings, with automatic reference counting for the latter. This allows for extremely
inexpensive passing and cloning of strings throughout your application, as no actual copying of
string data occurs when a `Str` is cloned or passed between functions.

By intelligently managing static references and reference counting, `Str` combines the best
of both worlds - the performance of static references and the flexibility of dynamic strings,
all with minimal overhead.

# Examples

```
use waterui_str::Str;

// Create from static string
let s1 = Str::from("static string");

// Create from owned string
let s2 = Str::from(String::from("owned string"));

// Clone is cheap for both variants - just copies a pointer, no string data
let s3 = s1.clone();
let s4 = s2.clone();

// Converting to String
let owned = s2.into_string();

// Static strings don't have reference counts
assert_eq!(s1.reference_count(), None);

// Owned strings have reference counts
assert_eq!(s4.reference_count(), Some(1));
```
