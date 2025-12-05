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

    /// Apply standard dynamic range color for this views.
    ///
    /// By default, `WaterUI` enables high dynamic range color for all views.
    ///
    /// However, in some cases, you may want to apply standard dynamic range color for cetain views,
    /// for instance, user avatar.
    #[derive(Debug)]
    pub struct StandardDynamicRange;
    impl MetadataKey for StandardDynamicRange {}

    impl StandardDynamicRange {
        /// Creates a new StandardDynamicRange metadata.
        pub const fn new() -> Self {
            Self
        }
    }

    /// Apply high dynamic range color for this views.
    ///
    /// By default, `WaterUI` already applies high dynamic range color for all views.
    ///
    /// But if your parent view applied `StandardDynamicRange` metadata, you would use this metadata to override it.
    #[derive(Debug)]
    pub struct HighDynamicRange;
    impl MetadataKey for HighDynamicRange {}

    impl HighDynamicRange {
        /// Creates a new HighDynamicRange metadata.
        pub const fn new() -> Self {
            Self
        }
    }
}
