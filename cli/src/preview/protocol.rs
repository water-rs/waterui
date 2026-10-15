//! Preview export symbol naming.

/// Convert a function path to preview export symbol.
///
/// Example:
/// - `sidebar` with crate `my-crate` -> `waterui_preview_my_crate_sidebar`
/// - `dashboard::admin::card_preview` with crate `my-crate`
///   -> `waterui_preview_my_crate_card_preview`
///
/// # Panics
///
/// Panics if `function_path` does not end with a function name.
#[must_use]
pub fn function_path_to_symbol(crate_name: &str, function_path: &str) -> String {
    // Replace dashes with underscores (Cargo uses dashes, Rust uses underscores)
    let crate_name = crate_name.replace('-', "_");
    let function_name = function_path
        .rsplit("::")
        .next()
        .expect("splitting a string always yields one segment");
    assert!(
        !function_name.is_empty(),
        "preview function path must end with a function name"
    );
    format!("waterui_preview_{crate_name}_{function_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_path_to_symbol() {
        assert_eq!(
            function_path_to_symbol("my_crate", "sidebar"),
            "waterui_preview_my_crate_sidebar"
        );

        assert_eq!(
            function_path_to_symbol("my-crate", "dashboard::admin::card_preview"),
            "waterui_preview_my_crate_card_preview"
        );
    }
}
