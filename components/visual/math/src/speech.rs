//! What a formula sounds like when it is read out.
//!
//! A formula reaches the screen as filled paths and glyph runs, so the node the
//! leaf publishes is the only place a screen reader can learn what it says.
//! `MathML` is the payload a platform's math accessibility API wants, but it is
//! markup rather than a sentence: read out, `<mfrac><mi>a</mi><mi>b</mi></mfrac>`
//! is noise. This module turns the markup [`crate::mathml`] publishes into the
//! sentence, through [`MathCAT`], the `ClearSpeak` / `MathSpeak` engine assistive
//! technology vendors use.
//!
//! `MathCAT` reads its speech rules from a `Rules` directory. This crate takes it
//! with the `include-zip` feature, which compiles the whole rule set into the
//! binary as one bzip2 archive and serves it from an in-memory filesystem — so a
//! formula speaks with nothing installed beside the application, and the rules
//! are not an asset anything has to carry, install, or find at runtime.
//!
//! The `Language` preference is left at `MathCAT`'s own default of `Auto`, which
//! it resolves itself.
//!
//! ```
//! use waterui_math::ast::MathStyle;
//! use waterui_math::{latex, mathml, speech};
//!
//! let formula = latex::parse(r"\frac{a}{b}")?;
//! let spoken = speech::speak(&mathml::to_mathml(&formula, MathStyle::Text))?;
//!
//! assert_eq!(spoken, "eigh over b");
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```
//!
//! [`MathCAT`]: https://nsoiffer.github.io/MathCAT/

use alloc::string::{String, ToString as _};
use core::cell::Cell;

/// Why a formula could not be spoken.
///
/// Every variant is a real failure with a name. There is deliberately no
/// "math formula" to say instead: a listener told only that a formula is
/// present has learnt nothing, and a generic announcement would hide the
/// failure behind something that sounds like it worked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpeechError {
    /// The rule set compiled into the binary could not be opened.
    #[error("the embedded MathCAT speech rules could not be read: {0}")]
    Rules(String),
    /// `MathCAT` refused the markup, or could not speak it.
    #[error("MathCAT could not speak the formula's MathML: {0}")]
    Markup(String),
    /// `MathCAT` accepted the markup and produced nothing.
    #[error("MathCAT produced no speech for the formula")]
    Silent,
}

/// The root the embedded rule archive is mounted at.
///
/// With `include-zip` this is a path inside the in-memory filesystem the crate
/// builds from its compiled-in archive, not a directory on disk.
const RULES_ROOT: &str = "Rules";

thread_local! {
    /// Whether this thread has already pointed `MathCAT` at the rule archive.
    ///
    /// `MathCAT` keeps its preference manager, its rule tables and the formula
    /// it is currently holding in thread-local storage, so "which rules are
    /// loaded" is a per-thread fact and this is the flag that matches it.
    /// Reading the rules again per formula would re-open the archive and
    /// re-parse the preference files on every accessibility emission.
    static RULES_READY: Cell<bool> = const { Cell::new(false) };
}

/// Speaks `mathml`.
///
/// # Errors
///
/// Returns [`SpeechError`] when the embedded rules cannot be read, when
/// `MathCAT` refuses the markup, or when it produces nothing to say.
pub fn speak(mathml: &str) -> Result<String, SpeechError> {
    RULES_READY.with(|ready| {
        if ready.get() {
            return Ok(());
        }
        libmathcat::set_rules_dir(RULES_ROOT.to_string())
            .map_err(|error| SpeechError::Rules(libmathcat::errors_to_string(&error)))?;
        ready.set(true);
        Ok(())
    })?;

    libmathcat::set_mathml(mathml.to_string())
        .map_err(|error| SpeechError::Markup(libmathcat::errors_to_string(&error)))?;
    let spoken = libmathcat::get_spoken_text()
        .map_err(|error| SpeechError::Markup(libmathcat::errors_to_string(&error)))?;

    let spoken = spoken.trim();
    if spoken.is_empty() {
        return Err(SpeechError::Silent);
    }
    Ok(spoken.to_string())
}

#[cfg(test)]
mod tests {
    use super::{SpeechError, speak};
    use crate::ast::MathStyle;
    use crate::{latex, mathml};

    fn spoken(source: &str) -> String {
        let item = latex::parse(source).unwrap_or_else(|error| panic!("`{source}`: {error}"));
        speak(&mathml::to_mathml(&item, MathStyle::Display))
            .unwrap_or_else(|error| panic!("`{source}`: {error}"))
    }

    /// The point of the whole module: a fraction is announced in words — "eigh
    /// over b", the letter spelled the way a speech engine has to spell it so a
    /// text-to-speech voice does not read `a` as the article — rather than as
    /// the markup that describes it.
    #[test]
    fn a_fraction_is_spoken_in_words() {
        let said = spoken(r"\frac{a}{b}");

        assert!(
            said.contains("over"),
            "a simple fraction is announced as one thing over another, got `{said}`"
        );
        assert!(
            !said.contains('<'),
            "speech is a sentence, not markup, got `{said}`"
        );
    }

    /// The formula from the issue this module exists for. Every landmark of it
    /// has to survive into the sentence, because a listener who hears only
    /// "fraction" cannot reconstruct the formula.
    #[test]
    fn the_quadratic_formula_keeps_all_of_its_parts() {
        let said = spoken(r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}").to_lowercase();

        for landmark in ["fraction", "square root", "plus or minus", "squared"] {
            assert!(
                said.contains(landmark),
                "the quadratic formula must be spoken with `{landmark}`, got `{said}`"
            );
        }
    }

    /// Markup that is not `MathML` at all is refused by name.
    #[test]
    fn something_that_is_not_mathml_is_refused() {
        assert!(matches!(
            speak("not markup at all"),
            Err(SpeechError::Markup(_))
        ));
    }
}
