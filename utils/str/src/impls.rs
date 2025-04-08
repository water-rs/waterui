use core::{
    convert::Infallible,
    fmt::Display,
    hash::Hash,
    mem::take,
    ops::{Add, AddAssign, Deref, Index},
    slice::SliceIndex,
    str::FromStr,
};

use alloc::string::{String, ToString};

use crate::Str;

impl Hash for Str {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.deref().eq(other.deref())
    }
}

impl<I: SliceIndex<str>> Index<I> for Str {
    type Output = <I as SliceIndex<str>>::Output;
    fn index(&self, index: I) -> &Self::Output {
        self.deref().index(index)
    }
}

impl FromStr for Str {
    type Err = Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s.to_string()))
    }
}

impl<S: AsRef<str>> FromIterator<S> for Str {
    fn from_iter<T: IntoIterator<Item = S>>(iter: T) -> Self {
        iter.into_iter()
            .fold(Str::new(), |state, s| state + s.as_ref())
    }
}

impl Eq for Str {}

impl PartialOrd for Str {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Str {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl Display for Str {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.deref().fmt(f)
    }
}

impl<'a> Extend<&'a str> for Str {
    fn extend<T: IntoIterator<Item = &'a str>>(&mut self, iter: T) {
        self.handle(move |string| {
            string.extend(iter);
        });
    }
}

impl Extend<String> for Str {
    fn extend<T: IntoIterator<Item = String>>(&mut self, iter: T) {
        self.handle(move |string| {
            string.extend(iter);
        });
    }
}

impl Extend<Str> for Str {
    fn extend<T: IntoIterator<Item = Str>>(&mut self, iter: T) {
        self.handle(move |string| {
            for s in iter.into_iter() {
                string.push_str(&s);
            }
        });
    }
}

impl<T> Add<T> for &Str
where
    T: AsRef<str>,
{
    type Output = Str;
    fn add(self, rhs: T) -> Self::Output {
        self.clone().add(rhs)
    }
}

impl<T> Add<T> for Str
where
    T: AsRef<str>,
{
    type Output = Self;
    fn add(self, rhs: T) -> Self::Output {
        let rhs = rhs.as_ref();
        (self.into_string() + rhs).into()
    }
}

impl<T> AddAssign<T> for Str
where
    T: AsRef<str>,
{
    fn add_assign(&mut self, rhs: T) {
        let rhs = rhs.as_ref();

        let string = take(self).into_string();
        *self = (string + rhs).into();
    }
}

#[cfg(feature = "serde")]
mod serde {
    use core::ops::Deref;

    use super::Str;
    use alloc::string::{String, ToString};
    use serde::{Deserialize, Deserializer, Serialize, de::Visitor};
    struct StrVisitor;

    impl Serialize for Str {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            self.deref().serialize(serializer)
        }
    }

    impl Visitor<'_> for StrVisitor {
        type Value = Str;

        fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
            formatter.write_str("a string")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
            Ok(v.to_string().into())
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
            Ok(v.into())
        }
    }

    impl<'de> Deserialize<'de> for Str {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_string(StrVisitor)
        }
    }
}
