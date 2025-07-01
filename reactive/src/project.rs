use crate::Binding;

/// Trait for projecting bindings into their component parts.
///
/// This trait allows decomposing complex bindings (like tuples or structs)
/// into separate bindings for each field, enabling granular reactive updates.
pub trait Project: Sized {
    /// The type resulting from projection (usually a tuple of bindings).
    type Projected;

    /// Creates projected bindings from a source binding.
    ///
    /// This method decomposes the source binding into separate bindings
    /// for each component, maintaining bidirectional reactivity.
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
    /// Projects this binding into its component parts.
    ///
    /// This method uses the `Project` trait implementation to decompose
    /// the binding into separate reactive bindings for each component.
    #[must_use]
    pub fn project(&self) -> T::Projected {
        T::project(self)
    }
}
