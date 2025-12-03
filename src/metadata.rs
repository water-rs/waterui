/// Secure metadata module.
pub mod secure {
    /// Secure metadata for secure fields.
    ///
    /// User would be forbidden to take a screenshot of the view that has this metadata.
    #[derive(Debug)]
    pub struct Secure;

    impl Secure {
        /// Creates a new Secure metadata.
        pub const fn new() -> Self {
            Self
        }
    }
}
