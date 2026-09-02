//! Reading LaTeX into the semantic tree.
//!
//! The parsing itself is `pulldown-latex`'s: it produces a flat stream of
//! events in which a construct is followed by the fixed number of items it
//! consumes, exactly like `pulldown-cmark`. This module's whole job is to fold
//! that stream into the tree in [`crate::ast`].
//!
//! Constructs this crate cannot yet lay out are reported as
//! [`LatexError::Unsupported`] naming the construct. They are not skipped:
//! silently dropping a matrix leaves a formula that is wrong in a way nobody
//! can see, which is worse than refusing it.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use pulldown_latex::Storage;
use pulldown_latex::event::{Content, DelimiterType, Event, Grouping, ScriptType, Visual};
use waterui_str::Str;

use crate::ast::{MathClass, MathItem, Operator};

/// Why a formula could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LatexError {
    /// The source is not well-formed LaTeX.
    #[error("could not parse the formula: {message}")]
    Parse {
        /// What the parser objected to.
        message: String,
    },
    /// The source is valid, but uses something this layout engine does not
    /// implement yet.
    #[error("`{construct}` is not supported yet")]
    Unsupported {
        /// The construct that was met.
        construct: &'static str,
    },
    /// A construct promised operands the stream did not deliver.
    #[error("the formula ended in the middle of `{construct}`")]
    Truncated {
        /// The construct that was left incomplete.
        construct: &'static str,
    },
}

/// Reads a LaTeX formula into the semantic tree.
///
/// # Errors
///
/// Returns [`LatexError`] when the source does not parse, or uses a construct
/// this engine does not implement.
pub fn parse(source: &str) -> Result<MathItem, LatexError> {
    let storage = Storage::new();
    let parser = pulldown_latex::Parser::new(source, &storage);

    let mut events = Vec::new();
    for event in parser {
        events.push(event.map_err(|error| LatexError::Parse {
            message: error.to_string(),
        })?);
    }

    let mut reader = Reader {
        events: &events,
        position: 0,
    };
    let mut items = Vec::new();
    while reader.position < reader.events.len() {
        items.push(reader.item()?);
    }
    Ok(MathItem::row(items))
}

struct Reader<'a, 'b> {
    events: &'a [Event<'b>],
    position: usize,
}

impl Reader<'_, '_> {
    /// Reads exactly one item: an atom, a group, or a construct with its
    /// operands.
    fn item(&mut self) -> Result<MathItem, LatexError> {
        // Taken by value so the reader stays free to advance while a construct
        // reads the operands that follow it.
        let Some(event) = self.events.get(self.position).cloned() else {
            return Err(LatexError::Truncated {
                construct: "expression",
            });
        };
        self.position += 1;

        match event {
            Event::Content(content) => Ok(atom(&content)),
            Event::Begin(grouping) => self.group(&grouping),
            Event::End => Err(LatexError::Parse {
                message: String::from("unbalanced group"),
            }),
            Event::Visual(visual) => self.visual(visual),
            Event::Script { ty, .. } => self.script(ty),
            Event::Space { width, .. } => Ok(MathItem::Space(width.map_or(0.0, em_of))),
            // A state change alters font variant or size for what follows.
            // Honouring it needs the mathematical-alphanumeric mapping for each
            // variant, which is a separate piece of work; ignoring it would
            // render bold as regular with nothing to say so.
            Event::StateChange(_) => Err(LatexError::Unsupported {
                construct: "font and style changes",
            }),
            Event::EnvironmentFlow(_) => Err(LatexError::Unsupported {
                construct: "multi-line and tabular environments",
            }),
        }
    }

    fn group(&mut self, grouping: &Grouping) -> Result<MathItem, LatexError> {
        match grouping {
            Grouping::Normal => {
                let items = self.until_end()?;
                Ok(MathItem::row(items))
            }
            Grouping::LeftRight(open, close) => {
                let items = self.until_end()?;
                Ok(MathItem::Fenced {
                    open: open.map(|character| fence(character, MathClass::Open)),
                    body: Box::new(MathItem::row(items)),
                    close: close.map(|character| fence(character, MathClass::Close)),
                })
            }
            Grouping::Array(_) | Grouping::Matrix { .. } | Grouping::SubArray { .. } => {
                Err(LatexError::Unsupported {
                    construct: "matrices and arrays",
                })
            }
            Grouping::Cases { .. } => Err(LatexError::Unsupported { construct: "cases" }),
            _ => Err(LatexError::Unsupported {
                construct: "multi-line and tabular environments",
            }),
        }
    }

    /// Reads items until the matching `End`.
    fn until_end(&mut self) -> Result<Vec<MathItem>, LatexError> {
        let mut items = Vec::new();
        loop {
            match self.events.get(self.position) {
                None => return Err(LatexError::Truncated { construct: "group" }),
                Some(Event::End) => {
                    self.position += 1;
                    return Ok(items);
                }
                Some(_) => items.push(self.item()?),
            }
        }
    }

    fn visual(&mut self, visual: Visual) -> Result<MathItem, LatexError> {
        match visual {
            Visual::SquareRoot => Ok(MathItem::Radical {
                radicand: Box::new(self.item()?),
                degree: None,
            }),
            Visual::Root => {
                // The stream gives the radicand first, then the index.
                let radicand = self.item()?;
                let degree = self.item()?;
                Ok(MathItem::Radical {
                    radicand: Box::new(radicand),
                    degree: Some(Box::new(degree)),
                })
            }
            Visual::Fraction(_) => {
                let numerator = self.item()?;
                let denominator = self.item()?;
                Ok(MathItem::Fraction {
                    numerator: Box::new(numerator),
                    denominator: Box::new(denominator),
                })
            }
            Visual::Negation => Err(LatexError::Unsupported {
                construct: "negation",
            }),
        }
    }

    fn script(&mut self, ty: ScriptType) -> Result<MathItem, LatexError> {
        let base = Box::new(self.item()?);
        match ty {
            ScriptType::Subscript => Ok(MathItem::Scripts {
                base,
                sub: Some(Box::new(self.item()?)),
                sup: None,
            }),
            ScriptType::Superscript => Ok(MathItem::Scripts {
                base,
                sub: None,
                sup: Some(Box::new(self.item()?)),
            }),
            ScriptType::SubSuperscript => {
                let sub = Box::new(self.item()?);
                let sup = Box::new(self.item()?);
                Ok(MathItem::Scripts {
                    base,
                    sub: Some(sub),
                    sup: Some(sup),
                })
            }
        }
    }
}

/// One content event as a leaf of the tree.
fn atom(content: &Content<'_>) -> MathItem {
    match content {
        Content::Text(text) => MathItem::Text(Str::from((*text).to_string())),
        Content::Number(number) => MathItem::Number(Str::from((*number).to_string())),
        // A function name is several letters that must not be read as a product
        // of variables, so it is upright, like literal text.
        Content::Function(name) => MathItem::Ident(Str::from((*name).to_string())),
        Content::Ordinary { content, stretchy } => {
            let glyph = Str::from(content.to_string());
            if *stretchy {
                MathItem::Operator(Operator::new(glyph, MathClass::Ord).stretchy())
            } else if content.is_alphabetic() {
                MathItem::Ident(glyph)
            } else {
                MathItem::Operator(Operator::new(glyph, MathClass::Ord))
            }
        }
        Content::LargeOp { content, .. } => {
            MathItem::Operator(Operator::new(Str::from(content.to_string()), MathClass::Op))
        }
        Content::BinaryOp { content, .. } => MathItem::Operator(Operator::new(
            Str::from(content.to_string()),
            MathClass::Bin,
        )),
        Content::Relation { content, .. } => MathItem::Operator(Operator::new(
            Str::from(relation_text(*content)),
            MathClass::Rel,
        )),
        Content::Delimiter { content, ty, .. } => {
            let class = match ty {
                DelimiterType::Open => MathClass::Open,
                DelimiterType::Close => MathClass::Close,
                DelimiterType::Fence => MathClass::Ord,
            };
            MathItem::Operator(Operator::new(Str::from(content.to_string()), class).stretchy())
        }
        Content::Punctuation(character) => MathItem::Operator(Operator::new(
            Str::from(character.to_string()),
            MathClass::Punct,
        )),
    }
}

fn fence(character: char, class: MathClass) -> Operator {
    Operator::new(Str::from(character.to_string()), class).stretchy()
}

/// A relation's characters.
///
/// `RelationContent` keeps its fields private and hands out UTF-8 through a
/// caller-supplied buffer. A relation is one character, or two stacked ones, so
/// eight bytes covers it with room to spare.
fn relation_text(content: pulldown_latex::event::RelationContent) -> String {
    let mut buffer = [0_u8; 8];
    let encoded = content.encode_utf8_to_buf(&mut buffer);
    core::str::from_utf8(encoded).map_or_else(|_| String::new(), ToString::to_string)
}

/// A dimension as a multiple of the em.
fn em_of(dimension: pulldown_latex::event::Dimension) -> f32 {
    use pulldown_latex::event::DimensionUnit;

    let value = dimension.value;
    match dimension.unit {
        DimensionUnit::Em => value,
        DimensionUnit::Ex => value * 0.5,
        // The remaining units are absolute; at the reference 10pt em they come
        // out as these multiples. A formula's own size scales the result, which
        // is why the conversion lands in ems rather than pixels.
        DimensionUnit::Pt => value / 10.0,
        DimensionUnit::Pc => value * 1.2,
        DimensionUnit::In => value * 7.227,
        DimensionUnit::Cm => value * 2.845,
        DimensionUnit::Mm => value * 0.2845,
        DimensionUnit::Bp => value * 0.1004,
        DimensionUnit::Dd => value * 0.107,
        DimensionUnit::Cc => value * 1.284,
        DimensionUnit::Sp => value / 655_360.0,
        DimensionUnit::Mu => value / 18.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::MathStyle;
    use crate::spacing::SpacingTable;

    fn parsed(source: &str) -> MathItem {
        parse(source).unwrap_or_else(|error| panic!("`{source}` must parse: {error}"))
    }

    #[test]
    fn a_fraction_becomes_a_fraction_node() {
        assert!(matches!(parsed(r"\frac{a}{b}"), MathItem::Fraction { .. }));
    }

    #[test]
    fn a_square_root_becomes_a_radical_without_a_degree() {
        let MathItem::Radical { degree, .. } = parsed(r"\sqrt{x}") else {
            panic!("expected a radical");
        };
        assert!(degree.is_none());
    }

    #[test]
    fn a_cube_root_carries_its_index() {
        let MathItem::Radical { degree, .. } = parsed(r"\sqrt[3]{x}") else {
            panic!("expected a radical");
        };
        assert!(
            degree.is_some(),
            "the index of a cube root must survive parsing"
        );
    }

    #[test]
    fn scripts_attach_to_their_base() {
        let MathItem::Scripts { sub, sup, .. } = parsed("x^2") else {
            panic!("expected scripts");
        };
        assert!(sup.is_some() && sub.is_none());

        let MathItem::Scripts { sub, sup, .. } = parsed("x_i^2") else {
            panic!("expected scripts");
        };
        assert!(sup.is_some() && sub.is_some());
    }

    /// The classes are the whole point of parsing into this tree rather than a
    /// list of characters: without them there is no spacing.
    #[test]
    fn operators_carry_the_class_that_drives_spacing() {
        let MathItem::Row(items) = parsed("a+b=c") else {
            panic!("expected a row");
        };
        let classes: Vec<_> = items.iter().map(MathItem::class).collect();
        assert!(
            classes.contains(&MathClass::Bin),
            "`+` must be a binary operator, got {classes:?}"
        );
        assert!(
            classes.contains(&MathClass::Rel),
            "`=` must be a relation, got {classes:?}"
        );
    }

    /// The gaps `a+b=c` gets must actually differ, end to end from source to
    /// spacing table. This is the regression that catches a formula rendered as
    /// one unbroken run.
    #[test]
    fn a_plus_b_equals_c_is_spaced_at_three_widths() {
        let MathItem::Row(items) = parsed("a+b=c") else {
            panic!("expected a row");
        };
        let table = SpacingTable::load().expect("spacing table loads");
        let gaps: Vec<f32> = items
            .windows(2)
            .map(|pair| table.between(pair[0].class(), pair[1].class(), MathStyle::Text))
            .collect();

        assert!(
            gaps.iter().any(|gap| *gap > 0.0),
            "at least one gap must be non-zero, got {gaps:?}"
        );
        let widest = gaps.iter().copied().fold(0.0_f32, f32::max);
        let narrowest = gaps.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            widest > narrowest,
            "the gaps around `+` and `=` must differ, got {gaps:?}"
        );
    }

    /// `\text{}` keeps its spaces and stays one node. Splitting it into
    /// characters and dropping the whitespace turns `\text{if } x` into an
    /// italic `ifx`.
    #[test]
    fn literal_text_keeps_its_spaces() {
        let item = parsed(r"\text{if }");
        let text = match &item {
            MathItem::Text(text) => text.as_str(),
            MathItem::Row(items) => match items.as_slice() {
                [MathItem::Text(text)] => text.as_str(),
                other => panic!("expected literal text, got {other:?}"),
            },
            other => panic!("expected literal text, got {other:?}"),
        };
        assert!(
            text.contains("if"),
            "the words inside \\text must survive, got {text:?}"
        );
        assert!(
            text.ends_with(' '),
            "the trailing space inside \\text must survive, got {text:?}"
        );
    }

    /// A multi-letter function name is one identifier, not a product of
    /// single-letter variables — so it is set upright.
    #[test]
    fn a_function_name_stays_one_identifier() {
        let item = parsed(r"\sin");
        assert!(
            matches!(&item, MathItem::Ident(name) if name.as_str().contains("sin")),
            "expected a single `sin` identifier, got {item:?}"
        );
    }

    /// Stretchy fences survive as a fenced group so they can grow later.
    #[test]
    fn left_right_becomes_a_fenced_group() {
        let item = parsed(r"\left(\frac{a}{b}\right)");
        let MathItem::Fenced { open, close, .. } = &item else {
            panic!("expected a fenced group, got {item:?}");
        };
        assert!(open.as_ref().is_some_and(|fence| fence.stretchy));
        assert!(close.as_ref().is_some_and(|fence| fence.stretchy));
    }

    /// Greek letters the old implementation was missing must simply work,
    /// because the symbol table is the parser's, not ours.
    #[test]
    fn greek_letters_outside_a_hand_written_table_parse() {
        for source in [r"\psi", r"\eta", r"\tau", r"\xi", r"\Upsilon", r"\zeta"] {
            parse(source).unwrap_or_else(|error| panic!("`{source}` must parse: {error}"));
        }
    }

    /// An unsupported construct is refused by name. It is not dropped: a
    /// silently missing matrix is a formula that is wrong with nothing to show
    /// for it.
    #[test]
    fn unsupported_constructs_are_refused_rather_than_dropped() {
        let error = parse(r"\begin{matrix}a & b\\c & d\end{matrix}")
            .expect_err("a matrix is not supported yet and must say so");
        assert!(
            matches!(error, LatexError::Unsupported { .. }),
            "expected an unsupported-construct error, got {error:?}"
        );
    }

    /// A parse failure is an error the caller can handle, never a panic: the
    /// source routinely comes from user or model output.
    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(parse(r"\frac{a}").is_err());
        assert!(parse(r"\nosuchcommand").is_err());
    }
}
