//! Integers that do not fit in a JavaScript number.
//!
//! JSON has one numeric type and JavaScript reads it as a double, so an integer
//! past 2^53 loses its low bits on the way into a page — silently, and in both
//! directions. `9007199254740993` seeded into a page reads back as
//! `...992`, and a page that assigns the value it was given returns the rounded
//! one, so a round trip through the page quietly corrupts state that never
//! looked wrong at either end.
//!
//! Rust's side of that boundary is `u64` and `i64`, which makes this ordinary
//! rather than exotic: an id, a file size, a nanosecond timestamp. JavaScript's
//! answer for those is `BigInt`, which JSON cannot spell, so one is carried
//! across as `{"__wateruiBigInt": "9007199254740993"}` and revived by the
//! bridge script.
//!
//! **Tagging follows the value, not the type.** A `u64` that fits stays an
//! ordinary number, so pages doing arithmetic on small values keep working and
//! only the values that were already being corrupted change shape. Deciding by
//! Rust type instead would turn every `usize` into a `BigInt` and break page
//! code that is perfectly correct today.

use serde_json::{Map, Value};

/// The key that marks a tagged integer. Also spelled in `js/bridge.js`.
const TAG: &str = "__wateruiBigInt";

/// `Number.MAX_SAFE_INTEGER`: the largest integer a double represents exactly.
const MAX_SAFE: u64 = 9_007_199_254_740_991;

fn is_beyond_double(value: &Value) -> bool {
    let Value::Number(number) = value else {
        return false;
    };
    if let Some(unsigned) = number.as_u64() {
        return unsigned > MAX_SAFE;
    }
    number
        .as_i64()
        .is_some_and(|signed| signed.unsigned_abs() > MAX_SAFE)
}

/// Whether serialized JSON could contain an integer a double cannot hold.
///
/// Going through a `serde_json::Value` to find out costs two to four times what
/// serializing the answer costs in the first place — building the tree, not
/// walking it — and almost every message has nothing to find. This reads the
/// bytes instead: every integer past 2^53 has at least 16 digits, so a shorter
/// run cannot be one, and a false positive (16 digits inside a string, or a
/// 16-digit value that does fit) only takes the slow path, which is correct
/// anyway.
pub fn might_contain_unrepresentable(json: &[u8]) -> bool {
    /// `9007199254740991` is 16 digits, and nothing longer than it fits.
    const SHORTEST: usize = 16;

    let mut run = 0_usize;
    for byte in json {
        if byte.is_ascii_digit() {
            run += 1;
            if run >= SHORTEST {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Whether a payload from a page carries a tagged integer.
///
/// Exact rather than a guess: the tag is a fixed key, so a payload without it
/// has nothing to untag and can be deserialized straight from its bytes.
pub fn contains_tag(json: &[u8]) -> bool {
    json.windows(TAG.len())
        .any(|window| window == TAG.as_bytes())
}

/// Replaces every integer a double cannot hold with its tagged form.
///
/// Walks the whole value: one of these can sit anywhere a number can, including
/// inside a struct field or an array element.
pub fn tag_unrepresentable(value: &mut Value) {
    if is_beyond_double(value) {
        let mut tagged = Map::new();
        tagged.insert(TAG.to_owned(), Value::String(value.to_string()));
        *value = Value::Object(tagged);
        return;
    }

    match value {
        Value::Array(items) => items.iter_mut().for_each(tag_unrepresentable),
        Value::Object(entries) => entries.values_mut().for_each(tag_unrepresentable),
        _ => {}
    }
}

/// Turns tagged integers back into numbers, for values arriving from a page.
///
/// A page that never touched the value returns the tag it was given; one that
/// built a `BigInt` itself is serialized into the same shape by the bridge
/// script. A tag whose payload is not an integer is left alone rather than
/// rejected: it arrives from page script, so it is untrusted, and the field's
/// own `Deserialize` is what should refuse it.
pub fn untag(value: &mut Value) {
    if let Value::Object(entries) = value
        && entries.len() == 1
        && let Some(Value::String(digits)) = entries.get(TAG)
        && let Ok(number) = digits.parse::<serde_json::Number>()
    {
        *value = Value::Number(number);
        return;
    }

    match value {
        Value::Array(items) => items.iter_mut().for_each(untag),
        Value::Object(entries) => entries.values_mut().for_each(untag),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{contains_tag, might_contain_unrepresentable, tag_unrepresentable, untag};
    use serde_json::{Value, json};

    fn tagged(value: Value) -> Value {
        let mut value = value;
        tag_unrepresentable(&mut value);
        value
    }

    fn untagged(value: Value) -> Value {
        let mut value = value;
        untag(&mut value);
        value
    }

    /// The whole point: a value a double cannot hold does not cross as a number.
    #[test]
    fn an_integer_past_the_safe_range_is_tagged() {
        assert_eq!(
            tagged(json!(9_007_199_254_740_993_u64)),
            json!({ "__wateruiBigInt": "9007199254740993" })
        );
        assert_eq!(
            tagged(json!(-9_007_199_254_740_993_i64)),
            json!({ "__wateruiBigInt": "-9007199254740993" })
        );
    }

    /// Everything a double holds exactly is left as a number, so page code doing
    /// arithmetic on ordinary values is untouched.
    #[test]
    fn values_a_double_holds_are_left_alone() {
        for value in [
            json!(0),
            json!(-1),
            json!(42),
            json!(9_007_199_254_740_991_u64),
            json!(-9_007_199_254_740_991_i64),
            json!(1.5),
            json!("9007199254740993"),
            json!(null),
        ] {
            assert_eq!(tagged(value.clone()), value, "{value} should not be tagged");
        }
    }

    /// A number can sit anywhere, so the walk has to reach all of it.
    #[test]
    fn tagging_reaches_nested_values() {
        assert_eq!(
            tagged(
                json!({ "sizes": [1, 9_007_199_254_740_993_u64], "id": { "raw": 9_007_199_254_740_993_u64 } })
            ),
            json!({
                "sizes": [1, { "__wateruiBigInt": "9007199254740993" }],
                "id": { "raw": { "__wateruiBigInt": "9007199254740993" } },
            })
        );
    }

    #[test]
    fn tagging_round_trips() {
        let original = json!({ "id": 9_007_199_254_740_993_u64, "small": 7 });
        assert_eq!(untagged(tagged(original.clone())), original);
    }

    /// The fast path must never say "nothing here" about a value that has
    /// something, or the tagging silently stops happening.
    #[test]
    fn the_prescan_never_misses_a_value_it_should_tag() {
        for value in [
            json!(9_007_199_254_740_992_u64),
            json!(u64::MAX),
            json!(i64::MIN),
            json!({ "nested": [ { "id": 9_007_199_254_740_993_u64 } ] }),
        ] {
            let bytes = serde_json::to_vec(&value).expect("serializes");
            assert!(
                might_contain_unrepresentable(&bytes),
                "{value} must reach the slow path"
            );
        }
    }

    /// And it should say so for ordinary messages, or it buys nothing.
    #[test]
    fn the_prescan_passes_over_ordinary_messages() {
        let bytes = serde_json::to_vec(&json!({
            "items": [{ "id": 1, "name": "item-1" }, { "id": 2_000_000, "name": "item-2" }],
            "when": 1_760_000_000_u64,
        }))
        .expect("serializes");
        assert!(!might_contain_unrepresentable(&bytes));
    }

    #[test]
    fn the_inbound_prescan_looks_for_the_tag_itself() {
        assert!(contains_tag(br#"{"id":{"__wateruiBigInt":"1"}}"#));
        assert!(!contains_tag(br#"{"id":12345678901234567890}"#));
        assert!(!contains_tag(b"{}"));
    }

    /// Page script reaches this, so a malformed tag must not be fatal, and must
    /// not be mistaken for a number either.
    #[test]
    fn a_tag_that_is_not_an_integer_is_left_for_the_field_to_reject() {
        for value in [
            json!({ "__wateruiBigInt": "not a number" }),
            json!({ "__wateruiBigInt": 5 }),
            json!({ "__wateruiBigInt": "1", "extra": 2 }),
        ] {
            assert_eq!(untagged(value.clone()), value);
        }
    }
}
