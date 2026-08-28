#[allow(
    clippy::module_inception,
    reason = "the `navigation` submodule holds the core navigation container; renaming would churn public paths"
)]
pub mod navigation;
pub mod tabs;
