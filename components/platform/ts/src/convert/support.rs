//! What `#[derive(TsType)]` and `#[derive(TsProps)]` expand into.
//!
//! The derives emit field-by-field conversions, and call these helpers for the
//! parts that would otherwise be repeated in every expansion: reading an
//! object's entries, converting a field and saying which one failed, and
//! decoding the adjacent tag an enum carrying data projects to. Nothing here
//! is a stable surface — it exists for the expansion — but it is documented,
//! because a macro's error messages are only as good as what they call.

use waterui_ts_engine::{JsError, JsValue};
use waterui_ts_schema::EnumRepresentation;

use crate::bridge::Bridge;
use crate::convert::FromJs;
use crate::error::kind_of;

/// The property an adjacently tagged enum carries its variant name under.
///
/// Read out of the representation the derive records in the schema, so the
/// conversion and the `.d.ts` can never disagree: if the default ever stopped
/// being a tagged representation, the `panic!` below would fail the build
/// instead.
pub const TAG: &str = match EnumRepresentation::DEFAULT_TAGGED {
    EnumRepresentation::Tagged { tag, .. } => tag,
    EnumRepresentation::StringUnion => panic!("DEFAULT_TAGGED is a tagged representation"),
};

/// The property an adjacently tagged enum carries its payload under.
pub const CONTENT: &str = match EnumRepresentation::DEFAULT_TAGGED {
    EnumRepresentation::Tagged { content, .. } => content,
    EnumRepresentation::StringUnion => panic!("DEFAULT_TAGGED is a tagged representation"),
};

/// The entries of the object a struct or a struct-shaped variant projects to.
///
/// # Errors
///
/// Returns [`JsError`] when the value is not an object, naming the type that
/// expected one.
pub fn entries<'a>(
    value: &'a JsValue,
    type_name: &str,
) -> Result<&'a [(String, JsValue)], JsError> {
    value.as_object().ok_or_else(|| {
        JsError::conversion(format!(
            "expected an object for {type_name}, found {}",
            kind_of(value)
        ))
    })
}

/// Converts one field of an object.
///
/// A property JavaScript left out reads as `undefined`, so an `Option` field
/// becomes `None` and every other type fails with its own message, prefixed
/// with the type and field that failed.
///
/// # Errors
///
/// Returns [`JsError`] when the property is missing or has the wrong shape.
pub fn field<T: FromJs>(
    entries: &[(String, JsValue)],
    type_name: &str,
    name: &str,
    bridge: &Bridge,
) -> Result<T, JsError> {
    let undefined = JsValue::Undefined;
    let value = entries
        .iter()
        .find(|(key, _)| key == name)
        .map_or(&undefined, |(_, value)| value);
    T::from_js(value, bridge).map_err(|error| JsError {
        message: format!("{type_name}.{name}: {}", error.message),
        ..error
    })
}

/// Refuses a property the schema does not declare.
///
/// A property nobody reads is not harmless: it is what the author wrote and
/// what they expect to have an effect, so dropping it turns a typo — `onClick`
/// for `onTap`, `opacity` where the modifier is spelled differently — into a
/// view that renders and does nothing. The error names what was given and what
/// is accepted, because that pair is the whole diagnosis.
///
/// # Errors
///
/// Returns [`JsError`] naming the first property `accepted` does not list.
pub fn reject_unknown(
    entries: &[(String, JsValue)],
    type_name: &str,
    accepted: &[&str],
) -> Result<(), JsError> {
    let Some((name, _)) = entries
        .iter()
        .find(|(key, _)| !accepted.contains(&key.as_str()))
    else {
        return Ok(());
    };
    Err(JsError::conversion(if accepted.is_empty() {
        format!("<{type_name}> takes no attributes, and was given `{name}`")
    } else {
        format!(
            "<{type_name}> has no `{name}` attribute. It accepts: {}",
            accepted.join(", ")
        )
    }))
}

/// Converts one positional field of a tuple variant.
///
/// # Errors
///
/// Returns [`JsError`] when the item has the wrong shape.
pub fn element<T: FromJs>(
    items: &[JsValue],
    type_name: &str,
    variant: &str,
    index: usize,
    bridge: &Bridge,
) -> Result<T, JsError> {
    let undefined = JsValue::Undefined;
    let value = items.get(index).unwrap_or(&undefined);
    T::from_js(value, bridge).map_err(|error| JsError {
        message: format!("{type_name}::{variant}.{index}: {}", error.message),
        ..error
    })
}

/// The variant name of an enum projected as a union of string literals.
///
/// # Errors
///
/// Returns [`JsError`] when the value is not a string.
pub fn variant_name<'a>(value: &'a JsValue, type_name: &str) -> Result<&'a str, JsError> {
    value.as_str().ok_or_else(|| {
        JsError::conversion(format!(
            "expected one of {type_name}'s variant names, found {}",
            kind_of(value)
        ))
    })
}

/// The tag and payload of an enum projected as an adjacently tagged object.
///
/// The payload is `None` for a unit variant, which carries none.
///
/// # Errors
///
/// Returns [`JsError`] when the value is not an object, or carries no string
/// under `tag`.
pub fn tagged<'a>(
    value: &'a JsValue,
    type_name: &str,
) -> Result<(&'a str, Option<&'a JsValue>), JsError> {
    let entries = entries(value, type_name)?;
    let find = |name: &str| {
        entries
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    let name = variant_name(
        find(TAG).ok_or_else(|| {
            JsError::conversion(format!(
                "expected {type_name} to carry its variant name under \"{TAG}\""
            ))
        })?,
        type_name,
    )?;
    Ok((name, find(CONTENT)))
}

/// The object an adjacently tagged variant projects to.
#[must_use]
pub fn tagged_object(variant: &str, payload: Option<JsValue>) -> JsValue {
    let mut entries = vec![(String::from(TAG), JsValue::String(variant.to_owned()))];
    if let Some(payload) = payload {
        entries.push((String::from(CONTENT), payload));
    }
    JsValue::Object(entries)
}

/// The payload of a variant that carries one.
///
/// # Errors
///
/// Returns [`JsError`] when the tagged object carries no payload.
pub fn payload<'a>(
    content: Option<&'a JsValue>,
    type_name: &str,
    variant: &str,
) -> Result<&'a JsValue, JsError> {
    content.ok_or_else(|| {
        JsError::conversion(format!(
            "{type_name}::{variant} carries data, so its object needs a \"{CONTENT}\" property"
        ))
    })
}

/// A variant name the enum does not have.
#[must_use]
pub fn unknown_variant(type_name: &str, name: &str) -> JsError {
    JsError::conversion(format!("{type_name} has no variant \"{name}\""))
}

/// The positional fields of a tuple variant, projected as an array.
///
/// # Errors
///
/// Returns [`JsError`] when the payload is not an array of exactly `arity`
/// items.
pub fn tuple_items<'a>(
    value: &'a JsValue,
    type_name: &str,
    variant: &str,
    arity: usize,
) -> Result<&'a [JsValue], JsError> {
    let items = value.as_array().ok_or_else(|| {
        JsError::conversion(format!(
            "expected an array of {arity} items for {type_name}::{variant}, found {}",
            kind_of(value)
        ))
    })?;
    if items.len() != arity {
        return Err(JsError::conversion(format!(
            "expected an array of {arity} items for {type_name}::{variant}, found {}",
            items.len()
        )));
    }
    Ok(items)
}
