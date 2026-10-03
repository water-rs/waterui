//! `Option`, sequences, and string-keyed maps.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use suiteki::Str;
use waterui_ts_engine::{JsError, JsValue};

use super::{FromJs, IntoJs, expected};
use crate::bridge::Bridge;

impl<T: IntoJs> IntoJs for Option<T> {
    /// `T | null`, as the schema declares it.
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        self.map_or_else(|| Ok(JsValue::Null), |value| value.into_js(bridge))
    }
}

impl<T: FromJs> FromJs for Option<T> {
    /// `null` is the absent value the schema declares; `undefined` reads the
    /// same way, because an optional property JavaScript simply left out
    /// arrives as `undefined` and means exactly the same thing.
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Null | JsValue::Undefined => Ok(None),
            present => T::from_js(present, bridge).map(Some),
        }
    }
}

impl<T: IntoJs> IntoJs for Vec<T> {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        let items = self
            .into_iter()
            .map(|item| item.into_js(bridge))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(JsValue::Array(items))
    }
}

impl<T: FromJs> FromJs for Vec<T> {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        value
            .as_array()
            .ok_or_else(|| expected("an array", value))?
            .iter()
            .map(|item| T::from_js(item, bridge))
            .collect()
    }
}

impl<T: IntoJs + Clone> IntoJs for &[T] {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        self.to_vec().into_js(bridge)
    }
}

impl<T: IntoJs, const N: usize> IntoJs for [T; N] {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        Vec::from(self).into_js(bridge)
    }
}

impl<T: FromJs, const N: usize> FromJs for [T; N] {
    /// The length is part of the Rust type and of the declared TypeScript
    /// type — `[T; N]` is the tuple `[T, T, …]`, not `T[]` — so an array of
    /// another length is an error naming both.
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        let items = value
            .as_array()
            .ok_or_else(|| expected("an array", value))?;
        if items.len() != N {
            return Err(JsError::conversion(format!(
                "expected an array of {N} items, the length this type declares, found {}",
                items.len()
            )));
        }
        let items: Vec<T> = items
            .iter()
            .map(|item| T::from_js(item, bridge))
            .collect::<Result<_, _>>()?;
        Self::try_from(items).map_err(|_| {
            JsError::conversion(format!("expected an array of {N} items after conversion"))
        })
    }
}

/// One map spelling: an object with the keys as properties, in key order.
macro_rules! string_keyed_maps {
    ($($map:ident < $key:ty $(, $hasher:ident)? >),* $(,)?) => {
        $(
            impl<V: IntoJs $(, $hasher: BuildHasher)?> IntoJs for $map<$key, V $(, $hasher)?> {
                fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
                    let mut entries: Vec<(String, JsValue)> = self
                        .into_iter()
                        .map(|(key, value)| Ok((key.as_str().to_owned(), value.into_js(bridge)?)))
                        .collect::<Result<_, JsError>>()?;
                    // A JavaScript object keeps insertion order; sorting makes
                    // the projection of a hashed map deterministic.
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                    Ok(JsValue::Object(entries))
                }
            }

            impl<V: FromJs $(, $hasher: BuildHasher + Default)?> FromJs for $map<$key, V $(, $hasher)?> {
                fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
                    value
                        .as_object()
                        .ok_or_else(|| expected("an object", value))?
                        .iter()
                        .map(|(key, value)| {
                            Ok((<$key>::from(key.clone()), V::from_js(value, bridge)?))
                        })
                        .collect::<Result<Self, JsError>>()
                }
            }
        )*
    };
}

string_keyed_maps!(
    BTreeMap<String>,
    BTreeMap<Str>,
    HashMap<String, S>,
    HashMap<Str, S>,
);
