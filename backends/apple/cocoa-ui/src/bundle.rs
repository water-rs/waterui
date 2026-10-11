//! The application bundle: its resources and its `Info.plist`.

use std::path::PathBuf;

use objc2_foundation::{NSBundle, NSString, NSURL};

/// The directory holding the application's resources, when the process runs
/// from a bundle that has one.
///
/// # Panics
///
/// If the bundle reports its resources at a location that is not a file
/// path, which no bundle loaded from disk does.
#[must_use]
pub fn resource_directory() -> Option<PathBuf> {
    NSBundle::mainBundle()
        .resourceURL()
        .map(|url| file_path(&url))
}

/// The path of the resource `name.extension` in the application bundle, or
/// `None` when the bundle has no such resource.
///
/// # Panics
///
/// If the bundle reports the resource at a location that is not a file path,
/// which no bundle loaded from disk does.
#[must_use]
pub fn resource(name: &str, extension: &str) -> Option<PathBuf> {
    NSBundle::mainBundle()
        .URLForResource_withExtension(
            Some(&NSString::from_str(name)),
            Some(&NSString::from_str(extension)),
        )
        .map(|url| file_path(&url))
}

/// The string the application's `Info.plist` holds under `key`, or `None`
/// when it has no such key.
///
/// Keys are looked up the way the system looks them up, so a localized
/// `InfoPlist.strings` value takes precedence over the plist itself.
///
/// # Panics
///
/// If the value under `key` is not a string: the bundle is then malformed for
/// every reader that expects one.
#[must_use]
pub fn info_string(key: &str) -> Option<String> {
    let value = NSBundle::mainBundle().objectForInfoDictionaryKey(&NSString::from_str(key))?;
    let string = value.downcast::<NSString>().unwrap_or_else(|value| {
        panic!("the Info.plist value under {key} is not a string: {value:?}")
    });
    Some(string.to_string())
}

fn file_path(url: &NSURL) -> PathBuf {
    url.to_file_path()
        .unwrap_or_else(|| panic!("the application bundle reported a non-file location: {url:?}"))
}
