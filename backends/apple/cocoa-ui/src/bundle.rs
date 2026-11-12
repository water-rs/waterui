//! The application bundle: its resources and its `Info.plist`.

use std::path::PathBuf;

use objc2_foundation::{NSBundle, NSString};
use waterui_contract::contract_violation;

/// The directory holding the application's resources, when the process runs
/// from a bundle that has one and reports its resources at a file path.
#[must_use]
pub fn resource_directory() -> Option<PathBuf> {
    NSBundle::mainBundle()
        .resourceURL()
        .and_then(|url| url.to_file_path())
}

/// The path of the resource `name.extension` in the application bundle, or
/// `None` when the bundle has no such resource or reports it at a location
/// that is not a file path.
#[must_use]
pub fn resource(name: &str, extension: &str) -> Option<PathBuf> {
    NSBundle::mainBundle()
        .URLForResource_withExtension(
            Some(&NSString::from_str(name)),
            Some(&NSString::from_str(extension)),
        )
        .and_then(|url| url.to_file_path())
}

/// The string the application's `Info.plist` holds under `key`, or `None`
/// when it has no such key.
///
/// Keys are looked up the way the system looks them up, so a localized
/// `InfoPlist.strings` value takes precedence over the plist itself.
///
/// A value under `key` that is not a string is a malformed bundle, reported as
/// a contract violation and answered as absent.
#[must_use]
pub fn info_string(key: &str) -> Option<String> {
    let value = NSBundle::mainBundle().objectForInfoDictionaryKey(&NSString::from_str(key))?;
    match value.downcast::<NSString>() {
        Ok(string) => Some(string.to_string()),
        Err(value) => {
            contract_violation!(
                "the Info.plist value under {key} is not a string: {value:?}; the key is treated as absent"
            );
            None
        }
    }
}
