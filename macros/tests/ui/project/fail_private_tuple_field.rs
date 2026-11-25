mod inner {
    use waterui::prelude::*;

    #[derive(Project, Clone)]
    pub struct SecretPair(pub u32, u32);

    impl SecretPair {
        pub const fn sample() -> Self {
            Self(1, 2)
        }
    }
}

fn main() {
    use waterui::{Binding, binding};

    let pair: Binding<inner::SecretPair> = binding(inner::SecretPair::sample());
    pair.project().1.set(99);
}
