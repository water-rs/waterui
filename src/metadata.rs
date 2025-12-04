/// Secure metadata module.
pub mod secure {
    use waterui_core::metadata::MetadataKey;

    /// Secure metadata for secure fields.
    ///
    /// User would be forbidden to take a screenshot of the view that has this metadata.
    #[derive(Debug)]
    pub struct Secure;

    impl MetadataKey for Secure {}

    impl Secure {
        /// Creates a new Secure metadata.
        pub const fn new() -> Self {
            Self
        }
    }
}
