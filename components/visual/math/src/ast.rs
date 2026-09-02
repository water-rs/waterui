//! The semantic tree a formula is parsed into.
//!
//! The shape follows `MathML` Core's element model rather than TeX's token
//! model, because `MathML` Core is what the layout algorithms in
//! [`crate::layout`] are specified against, and because the same tree is what
//! the accessibility layer publishes.
//!
//! The tree carries no font, no size and no position. Everything that depends
//! on the chosen face lives in [`crate::font`], and everything that depends on
//! a size lives in [`crate::layout`].

use alloc::boxed::Box;
use alloc::vec::Vec;

use waterui_str::Str;

/// How an atom relates to its neighbours.
///
/// This is the single most important thing the tree carries that a plain list
/// of characters does not: inter-atom spacing is a function of the classes of
/// the two atoms either side of the gap, so `a+b` and `a=b` are spaced
/// differently and `f(x)` has no gap before the parenthesis. Dropping the class
/// and concatenating advance widths produces a formula that is legible only by
/// accident.
///
/// The classes are `MathML` Core's, which are TeX's with `Inner` folded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathClass {
    /// An ordinary atom: a variable, a number, most symbols.
    #[default]
    Ord,
    /// A large operator, such as a summation or an integral sign.
    Op,
    /// A binary operator, such as `+`. Spaced on both sides, but only in
    /// contexts where a binary reading is possible.
    Bin,
    /// A relation, such as `=` or `<`. The widest spacing.
    Rel,
    /// An opening fence.
    Open,
    /// A closing fence.
    Close,
    /// Punctuation, such as the comma between arguments.
    Punct,
    /// A subformula treated as a unit, such as a fenced group.
    Inner,
}

/// The four TeX/MathML layout styles.
///
/// The style selects which of the paired OpenType `MATH` constants applies —
/// several come in a display and a non-display flavour — and how far scripts
/// scale down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum MathStyle {
    /// A formula set on its own line.
    Display,
    /// A formula set in running text.
    #[default]
    Text,
    /// First-level script.
    Script,
    /// Second-level script and beyond.
    ScriptScript,
}

impl MathStyle {
    /// The style a superscript, subscript or index takes inside `self`.
    #[must_use]
    pub const fn script(self) -> Self {
        match self {
            Self::Display | Self::Text => Self::Script,
            Self::Script | Self::ScriptScript => Self::ScriptScript,
        }
    }

    /// The style a fraction's numerator and denominator take inside `self`.
    #[must_use]
    pub const fn fraction(self) -> Self {
        match self {
            Self::Display => Self::Text,
            Self::Text => Self::Script,
            Self::Script | Self::ScriptScript => Self::ScriptScript,
        }
    }

    /// Whether the display flavour of a paired `MATH` constant applies.
    #[must_use]
    pub const fn is_display(self) -> bool {
        matches!(self, Self::Display)
    }

    /// Whether inter-atom spacing beyond a thin space is suppressed.
    ///
    /// TeX drops medium and thick spacing in script styles, which is why a
    /// superscripted sum does not acquire the gaps its display form has.
    #[must_use]
    pub const fn is_cramped_spacing(self) -> bool {
        matches!(self, Self::Script | Self::ScriptScript)
    }
}

/// An operator, relation, fence or punctuation mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    /// The character(s) drawn.
    pub glyph: Str,
    /// How it spaces against its neighbours.
    pub class: MathClass,
    /// Whether it grows to match the thing it encloses or spans.
    ///
    /// Fences around a fraction stretch; a `+` never does.
    pub stretchy: bool,
}

impl Operator {
    /// An operator of the given class, not stretchy.
    #[must_use]
    pub fn new(glyph: impl Into<Str>, class: MathClass) -> Self {
        Self {
            glyph: glyph.into(),
            class,
            stretchy: false,
        }
    }

    /// The same operator, marked as growing with its content.
    #[must_use]
    pub const fn stretchy(mut self) -> Self {
        self.stretchy = true;
        self
    }
}

/// One node of a formula.
#[derive(Debug, Clone, PartialEq)]
pub enum MathItem {
    /// A variable name. `MathML` `<mi>`; set in italic when it is a single
    /// letter, which is the convention the layer above applies.
    Ident(Str),
    /// A numeric literal. `MathML` `<mn>`; always upright.
    Number(Str),
    /// An operator, relation, fence or punctuation mark. `MathML` `<mo>`.
    Operator(Operator),
    /// Literal text, set upright with its spaces intact. `MathML` `<mtext>`.
    ///
    /// This is what `\text{}` produces. Keeping it a distinct node is what
    /// stops "if" from being read as the product of two variables and from
    /// losing the space after it.
    Text(Str),
    /// A horizontal sequence. `MathML` `<mrow>`.
    Row(Vec<Self>),
    /// A fraction. `MathML` `<mfrac>`.
    Fraction {
        /// The expression above the rule.
        numerator: Box<Self>,
        /// The expression below the rule.
        denominator: Box<Self>,
    },
    /// A radical, with an optional degree. `MathML` `<msqrt>` / `<mroot>`.
    Radical {
        /// What sits under the bar.
        radicand: Box<Self>,
        /// The index, as in a cube root. `None` is a square root.
        degree: Option<Box<Self>>,
    },
    /// Scripts attached to a base. `MathML` `<msub>` / `<msup>` / `<msubsup>`.
    Scripts {
        /// What the scripts attach to.
        base: Box<Self>,
        /// The subscript, if any.
        sub: Option<Box<Self>>,
        /// The superscript, if any.
        sup: Option<Box<Self>>,
    },
    /// Explicit horizontal space, in ems.
    ///
    /// This is what `\,` and its relatives produce. It is distinct from the
    /// automatic inter-atom spacing in [`crate::spacing`]: that is a property
    /// of the classes either side of a gap, this is an author's instruction.
    Space(f32),
    /// A group held between fences that grow to fit it.
    Fenced {
        /// The opening fence, absent for a half-open group.
        open: Option<Operator>,
        /// The enclosed expression.
        body: Box<Self>,
        /// The closing fence, absent for a half-open group.
        close: Option<Operator>,
    },
}

impl MathItem {
    /// An empty row, which is what an empty formula parses to.
    #[must_use]
    pub const fn empty() -> Self {
        Self::Row(Vec::new())
    }

    /// Wraps a sequence, collapsing the one-element case.
    ///
    /// A row of one is indistinguishable from its only child for both layout
    /// and accessibility, and keeping the wrapper would make every script base
    /// a nested row.
    #[must_use]
    pub fn row(mut items: Vec<Self>) -> Self {
        if items.len() == 1 {
            items.pop().unwrap_or_else(Self::empty)
        } else {
            Self::Row(items)
        }
    }

    /// How this node spaces against its neighbours.
    ///
    /// Only an operator carries a class of its own; everything else is either
    /// ordinary or, when it is a fenced group, an inner subformula.
    #[must_use]
    pub fn class(&self) -> MathClass {
        match self {
            Self::Operator(operator) => operator.class,
            Self::Fenced { .. } => MathClass::Inner,
            Self::Row(items) => match items.as_slice() {
                [only] => only.class(),
                _ => MathClass::Ord,
            },
            _ => MathClass::Ord,
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn a_row_of_one_collapses_to_its_child() {
        let collapsed = MathItem::row(vec![MathItem::Number(Str::from_static("1"))]);
        assert_eq!(collapsed, MathItem::Number(Str::from_static("1")));
    }

    #[test]
    fn an_empty_row_stays_a_row() {
        assert_eq!(MathItem::row(Vec::new()), MathItem::Row(Vec::new()));
    }

    /// A single-child row must not hide the class of what it wraps, or the
    /// operator inside it would be spaced as an ordinary atom.
    #[test]
    fn a_wrapped_operator_keeps_its_class() {
        let wrapped = MathItem::Row(vec![MathItem::Operator(Operator::new("=", MathClass::Rel))]);
        assert_eq!(wrapped.class(), MathClass::Rel);
    }

    #[test]
    fn styles_step_down_for_scripts_and_bottom_out() {
        assert_eq!(MathStyle::Display.script(), MathStyle::Script);
        assert_eq!(MathStyle::Text.script(), MathStyle::Script);
        assert_eq!(MathStyle::Script.script(), MathStyle::ScriptScript);
        assert_eq!(
            MathStyle::ScriptScript.script(),
            MathStyle::ScriptScript,
            "script style must bottom out rather than shrink without limit"
        );
    }

    /// A display fraction sets its parts in text style, not display style —
    /// otherwise nested fractions never shrink and a continued fraction is
    /// drawn at full size all the way down.
    #[test]
    fn fraction_parts_step_down_from_display() {
        assert_eq!(MathStyle::Display.fraction(), MathStyle::Text);
        assert_eq!(MathStyle::Text.fraction(), MathStyle::Script);
        assert_eq!(MathStyle::Script.fraction(), MathStyle::ScriptScript);
    }
}
