use waterui_core::Str;

pub struct Url(Str);

impl From<Str> for Url {
    fn from(value: Str) -> Self {
        Self(value)
    }
}

impl From<&'static str> for Url {
    fn from(value: &'static str) -> Self {
        Self(value.into())
    }
}

impl Url {
    pub fn new(url: impl Into<Str>) -> Self {
        Self(url.into())
    }
}
