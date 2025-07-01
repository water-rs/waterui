use crate::Binding;

pub trait Project: Sized {
    type Projected;
    fn project(source: &Binding<Self>) -> Self::Projected;
}

impl<T0: 'static, T1: 'static> Project for (T0, T1) {
    type Projected = (Binding<T0>, Binding<T1>);
    fn project(source: &Binding<Self>) -> Self::Projected {
        let t0: Binding<T0> = Binding::mapping(
            source,
            |value| value.0,
            |binding, value| {
                binding.get_mut().0 = value;
            },
        );
        let t1: Binding<T1> = Binding::mapping(
            source,
            |value| value.1,
            |binding, value| {
                binding.get_mut().1 = value;
            },
        );
        (t0, t1)
    }
}

impl<T: Project> Binding<T> {
    pub fn project(&self) -> T::Projected {
        T::project(self)
    }
}
