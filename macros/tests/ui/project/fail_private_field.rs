mod inner {
    use waterui::{Project, Str};

    #[derive(Project, Clone)]
    pub struct Account {
        pub name: Str,
        token: Str,
    }

    impl Account {
        pub fn sample() -> Self {
            Self {
                name: Str::from("Ada"),
                token: Str::from("secret"),
            }
        }
    }
}

fn main() {
    use waterui::{Binding, Str, binding};

    let account: Binding<inner::Account> = binding(inner::Account::sample());
    account.project().token.set(Str::from("forged"));
}
