use core::any::type_name;

use crate::{Environment, View};

#[derive(Debug)]
pub struct Native<T>(pub T);

impl<T: 'static> View for Native<T> {
    fn body(self, _env: &Environment) -> impl View {
        panic!("Native view ({})", type_name::<T>())
    }
}
