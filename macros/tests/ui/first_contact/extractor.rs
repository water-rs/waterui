//! `use_env(|client: ApiClient| ..)`: the parameter type must implement
//! `Extractor`; a plain type does not until `#[state]` or `impl_extractor!` marks it.

use waterui::env::use_env;
use waterui::prelude::*;

struct ApiClient;

fn broken() -> impl View {
    use_env(|_client: ApiClient| text!("offline"))
}

fn main() {}
