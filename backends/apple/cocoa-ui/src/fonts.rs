//! Making font files available to the process.
//!
//! # Safety
//!
//! The `unsafe` here is one Core Text call: its error out-parameter is a local
//! the call either leaves null or fills with a `CFError` it hands over under
//! the create rule, which is taken into a `CFRetained` exactly once.

use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};

use objc2_core_foundation::{CFError, CFRetained, CFURL};
use objc2_core_text::{CTFontManagerRegisterFontsForURL, CTFontManagerScope};

/// Registers every font in the file at `path` for this process, so text can
/// name the font's family from then on.
///
/// The registration lasts until the process exits, and the file must stay
/// where it is for that long. A font collection (`.ttc`, `.otc`) registers
/// each font it contains.
///
/// # Errors
///
/// If `path` cannot be expressed as a file URL, or Core Text refuses the
/// file: it is missing, is not a font, or holds a font already registered.
/// The error carries Core Text's own description of the failure.
///
/// # Panics
///
/// If Core Text refuses the file without describing why, which it documents
/// it never does.
pub fn register_font(path: &Path) -> Result<(), FontRegistrationError> {
    let url = CFURL::from_file_path(path).ok_or_else(|| FontRegistrationError {
        path: path.to_path_buf(),
        reason: String::from("the path cannot be expressed as a file URL"),
    })?;
    let mut error: *mut CFError = ptr::null_mut();
    // SAFETY: see the module safety note.
    let registered = unsafe {
        CTFontManagerRegisterFontsForURL(&url, CTFontManagerScope::Process, &raw mut error)
    };
    if registered {
        return Ok(());
    }
    let error =
        NonNull::new(error).expect("Core Text must describe why it refused to register a font");
    // SAFETY: Core Text returned the error under the create rule; see the
    // module safety note.
    let error = unsafe { CFRetained::from_raw(error) };
    Err(FontRegistrationError {
        path: path.to_path_buf(),
        reason: error
            .description()
            .expect("CFErrorCopyDescription is documented never to return null")
            .to_string(),
    })
}

/// Why a font file could not be registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontRegistrationError {
    path: PathBuf,
    reason: String,
}

impl FontRegistrationError {
    /// The font file that was not registered.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Core Text's description of the failure.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for FontRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "failed to register the font at {}: {}",
            self.path.display(),
            self.reason
        )
    }
}

impl std::error::Error for FontRegistrationError {}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::register_font;

    #[test]
    fn reports_core_texts_reason_for_a_missing_file() {
        let path = Path::new("/nonexistent/cocoa-ui-test-font.ttf");
        let error = register_font(path).expect_err("a missing file cannot be registered");
        assert_eq!(error.path(), path);
        assert!(!error.reason().is_empty(), "Core Text gave no description");
        assert!(
            error.to_string().starts_with(&format!(
                "failed to register the font at {}: ",
                path.display()
            )),
            "{error}"
        );
    }
}
