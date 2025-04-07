use crate::Binding;

#[derive(Debug, Clone)]
pub struct Focused(pub Binding<bool>);

impl Focused {
    pub fn new<T: 'static + Eq + Clone>(value: Binding<Option<T>>, equals: T) -> Self {
        Self(Binding::mapping(
            &value,
            {
                let equals = equals.clone();
                move |value| value.as_ref().filter(|value| **value == equals).is_some()
            },
            move |binding, value| {
                if value {
                    binding.set(Some(equals.clone()));
                }
            },
        ))
    }
}
