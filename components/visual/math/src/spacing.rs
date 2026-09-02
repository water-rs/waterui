//! Inter-atom spacing, read from `spacing.toml`.
//!
//! The gap between two adjacent atoms is a function of the class on each side
//! and the current style. Getting this wrong is not subtle once you look for
//! it: a layout that concatenates advance widths renders `a+b=c` as one
//! unbroken run, which is the single most visible way a formula can be wrong
//! while every glyph in it is correct.
//!
//! The table is data, so it lives in TOML. It is parsed once per formula into
//! a fixed-size array and then indexed, rather than being consulted as text.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use serde::Deserialize;

use crate::ast::{MathClass, MathStyle};

/// One math unit as a fraction of an em. TeX's `18mu = 1em`.
const MU: f32 = 1.0 / 18.0;

/// How many classes the table is square over.
const CLASS_COUNT: usize = 8;

/// A gap, before the style is taken into account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spacing {
    /// Width in math units.
    mu: u8,
    /// Whether script styles drop it.
    script_suppressed: bool,
}

impl Spacing {
    /// The gap in ems at `style`.
    fn em(self, style: MathStyle) -> f32 {
        if self.script_suppressed && style.is_cramped_spacing() {
            0.0
        } else {
            f32::from(self.mu) * MU
        }
    }
}

/// Why the shipped spacing table could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpacingError {
    /// The TOML did not parse.
    #[error("the spacing table is not valid TOML: {message}")]
    Malformed {
        /// What the parser objected to.
        message: String,
    },
    /// A row was missing, or the wrong length.
    #[error("the spacing table is not square over {CLASS_COUNT} classes: {detail}")]
    NotSquare {
        /// Which row is wrong and how.
        detail: String,
    },
    /// A cell named an amount the table does not define.
    #[error("the spacing table row `{row}` has unknown amount `{amount}`")]
    UnknownAmount {
        /// The row the bad cell is in.
        row: String,
        /// The text that was not recognised.
        amount: String,
    },
}

#[derive(Deserialize)]
struct RawTable {
    order: Vec<String>,
    rows: BTreeMap<String, Vec<String>>,
}

/// The spacing table, indexed by the classes either side of a gap.
#[derive(Debug, Clone)]
pub struct SpacingTable {
    cells: [[Spacing; CLASS_COUNT]; CLASS_COUNT],
}

impl SpacingTable {
    /// Reads the table shipped with this crate.
    ///
    /// # Errors
    ///
    /// [`SpacingError`] if `spacing.toml` does not parse, is not square over
    /// the eight classes, or names an amount the table does not define. All
    /// three are defects in this crate's own data, not in caller input.
    pub fn load() -> Result<Self, SpacingError> {
        let raw: RawTable = toml::from_str(include_str!("spacing.toml")).map_err(|error| {
            SpacingError::Malformed {
                message: error.to_string(),
            }
        })?;

        if raw.order.len() != CLASS_COUNT {
            return Err(SpacingError::NotSquare {
                detail: alloc::format!("`order` names {} classes", raw.order.len()),
            });
        }

        let mut cells = [[Spacing {
            mu: 0,
            script_suppressed: false,
        }; CLASS_COUNT]; CLASS_COUNT];
        for (row_index, row_name) in raw.order.iter().enumerate() {
            let row = raw
                .rows
                .get(row_name)
                .ok_or_else(|| SpacingError::NotSquare {
                    detail: alloc::format!("no row for class `{row_name}`"),
                })?;
            if row.len() != CLASS_COUNT {
                return Err(SpacingError::NotSquare {
                    detail: alloc::format!("row `{row_name}` has {} cells", row.len()),
                });
            }
            for (column, cell) in row.iter().enumerate() {
                cells[row_index][column] = parse_cell(row_name, cell)?;
            }
        }

        Ok(Self { cells })
    }

    /// The gap between an atom of class `left` and one of class `right`, in
    /// ems, at `style`.
    #[must_use]
    pub fn between(&self, left: MathClass, right: MathClass, style: MathStyle) -> f32 {
        self.cells[class_index(left)][class_index(right)].em(style)
    }
}

fn parse_cell(row: &str, cell: &str) -> Result<Spacing, SpacingError> {
    let (name, script_suppressed) = cell
        .strip_suffix('*')
        .map_or((cell, false), |base| (base, true));
    let mu = match name {
        "none" => 0,
        "thin" => 3,
        "medium" => 4,
        "thick" => 5,
        other => {
            return Err(SpacingError::UnknownAmount {
                row: String::from(row),
                amount: String::from(other),
            });
        }
    };
    Ok(Spacing {
        mu,
        script_suppressed,
    })
}

/// The table's column order, which `spacing.toml` states explicitly and this
/// mirrors.
const fn class_index(class: MathClass) -> usize {
    match class {
        MathClass::Ord => 0,
        MathClass::Op => 1,
        MathClass::Bin => 2,
        MathClass::Rel => 3,
        MathClass::Open => 4,
        MathClass::Close => 5,
        MathClass::Punct => 6,
        MathClass::Inner => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> SpacingTable {
        SpacingTable::load().expect("the shipped spacing table must load")
    }

    /// Well under the smallest gap the table can produce (3mu, or 1/6 em), so
    /// it separates "no gap" from "a gap" without asserting on exact float
    /// equality.
    const TOLERANCE: f32 = 1e-6;

    fn assert_no_gap(actual: f32, message: &str) {
        assert!(
            actual.abs() < TOLERANCE,
            "{message}: expected no gap, got {actual}"
        );
    }

    fn assert_same_gap(left: f32, right: f32, message: &str) {
        assert!(
            (left - right).abs() < TOLERANCE,
            "{message}: {left} and {right} should be the same gap"
        );
    }

    /// The shipped table must be well formed; this is the test that turns a
    /// typo in `spacing.toml` into a failure rather than a mis-spaced formula.
    #[test]
    fn the_shipped_table_loads() {
        let _ = table();
    }

    /// The reason the table exists: `a+b` and `a=b` are not spaced alike, and
    /// neither is spaced like `ab`.
    #[test]
    fn binary_relation_and_ordinary_gaps_differ() {
        let table = table();
        let ordinary = table.between(MathClass::Ord, MathClass::Ord, MathStyle::Text);
        let binary = table.between(MathClass::Ord, MathClass::Bin, MathStyle::Text);
        let relation = table.between(MathClass::Ord, MathClass::Rel, MathStyle::Text);

        assert_no_gap(ordinary, "adjacent ordinary atoms");
        assert!(
            binary > ordinary,
            "a binary operator must be spaced away from its left operand"
        );
        assert!(
            relation > binary,
            "a relation must be spaced more widely than a binary operator, \
             got relation={relation} binary={binary}"
        );
    }

    /// `f(x)` must close up: nothing is inserted before an opening fence.
    #[test]
    fn an_opening_fence_does_not_take_a_gap() {
        let table = table();
        assert_no_gap(
            table.between(MathClass::Ord, MathClass::Open, MathStyle::Text),
            "before an opening fence",
        );
        assert_no_gap(
            table.between(MathClass::Open, MathClass::Ord, MathStyle::Text),
            "after an opening fence",
        );
    }

    /// Script styles drop the wide gaps, which is why a superscripted sum is
    /// not set with the gaps its display form has.
    #[test]
    fn wide_gaps_vanish_in_script_styles() {
        let table = table();
        for style in [MathStyle::Script, MathStyle::ScriptScript] {
            assert_no_gap(
                table.between(MathClass::Ord, MathClass::Rel, style),
                &alloc::format!("{style:?} must suppress the relation gap"),
            );
            assert_no_gap(
                table.between(MathClass::Ord, MathClass::Bin, style),
                &alloc::format!("{style:?} must suppress the binary gap"),
            );
        }
    }

    /// A gap that is not marked suppressed survives into script styles.
    #[test]
    fn unsuppressed_gaps_survive_script_styles() {
        let table = table();
        let text = table.between(MathClass::Ord, MathClass::Op, MathStyle::Text);
        let script = table.between(MathClass::Ord, MathClass::Op, MathStyle::Script);
        assert!(text > 0.0);
        assert_same_gap(text, script, "an unsuppressed gap in text and script style");
    }

    /// The table is not symmetric, and must not be "tidied" into symmetry:
    /// a closing fence followed by a binary operator is spaced, the reverse
    /// is not.
    #[test]
    fn the_table_is_deliberately_asymmetric() {
        let table = table();
        assert!(table.between(MathClass::Close, MathClass::Bin, MathStyle::Text) > 0.0);
        assert_no_gap(
            table.between(MathClass::Bin, MathClass::Close, MathStyle::Text),
            "a binary operator before a closing fence",
        );
    }
}
