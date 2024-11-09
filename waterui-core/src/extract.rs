use core::any::type_name;

use crate::Environment;
use alloc::format;
use anyhow::Error;
pub trait Extractor: 'static + Sized {
    fn extract(env: &Environment) -> Result<Self, Error>;
}

pub struct Use<T: 'static>(pub T);

impl Extractor for Environment {
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(env.clone())
    }
}

impl<T: Extractor> Extractor for Option<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(Extractor::extract(env).ok())
    }
}

impl<T: 'static + Clone> Extractor for Use<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        if let Some(value) = env.get::<T>() {
            Ok(Self(value.clone()))
        } else {
            Err(Error::msg(format!(
                "Environment value `{}` not found",
                type_name::<T>()
            )))
        }
    }
}
