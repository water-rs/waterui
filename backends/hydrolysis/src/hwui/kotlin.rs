//! The Kotlin half of the wire, generated from the Rust definitions.
//!
//! `templates/hwui/Protocol.kt` renders the opcodes, every code table and
//! the text wire's constants into the `Protocol.kt` the Kotlin replayer and
//! text provider compile against. With `HWUI_KOTLIN_DIR` set, as the
//! `:hwui` Gradle module's `contractScenes` task sets it, the source is
//! written there; nothing generated is committed.

use std::fs;
use std::path::Path;

use askama::Template;

use super::protocol::{
    self, HEADER_WORDS, MAGIC, MATRIX_WORDS, MESH_BAND_PATCHES, MESH_PATCH_FLOATS, NONE, Op,
    PAINT_WORDS, SHAPE_WORDS, TRANSFORM_VALUES,
};
use super::text::wire;

/// A module of wire codes, as `codes!` declares it.
pub struct Codes<T: 'static> {
    /// The Kotlin object the codes go in.
    pub object: &'static str,
    /// The module's doc lines.
    pub doc: &'static [&'static str],
    /// Each code's name, value and doc lines.
    pub entries: &'static [(&'static str, T, &'static [&'static str])],
}

/// A Rust constant as a Kotlin `const val`.
trait Kotlin: Copy {
    const TYPE: &'static str;
    fn literal(self) -> String;
}

impl Kotlin for u32 {
    const TYPE: &'static str = "Int";
    fn literal(self) -> String {
        // The wire's words are `u32`s and Kotlin's `Int` is their bit pattern.
        i32::from_ne_bytes(self.to_ne_bytes()).to_string()
    }
}

impl Kotlin for u16 {
    const TYPE: &'static str = "Int";
    fn literal(self) -> String {
        self.to_string()
    }
}

impl Kotlin for i32 {
    const TYPE: &'static str = "Int";
    fn literal(self) -> String {
        self.to_string()
    }
}

impl Kotlin for usize {
    const TYPE: &'static str = "Int";
    fn literal(self) -> String {
        i32::try_from(self)
            .expect("a wire constant fits an Int")
            .to_string()
    }
}

impl Kotlin for f32 {
    const TYPE: &'static str = "Float";
    fn literal(self) -> String {
        assert!(self.is_finite(), "a wire float constant is finite");
        format!("{self:?}f")
    }
}

impl Kotlin for char {
    const TYPE: &'static str = "Char";
    fn literal(self) -> String {
        let unit = u16::try_from(u32::from(self)).expect("a wire char is one UTF-16 unit");
        format!("'\\u{unit:04x}'")
    }
}

struct Constant {
    name: &'static str,
    ty: &'static str,
    value: String,
    doc: String,
}

impl Constant {
    fn new<T: Kotlin>(name: &'static str, value: T, doc: &[&str]) -> Self {
        Self {
            name,
            ty: T::TYPE,
            value: value.literal(),
            doc: kdoc(doc),
        }
    }
}

struct Object {
    name: &'static str,
    doc: String,
    constants: Vec<Constant>,
}

impl Object {
    fn new<T: Kotlin>(codes: &Codes<T>) -> Self {
        Self {
            name: codes.object,
            doc: kdoc(codes.doc),
            constants: constants(codes),
        }
    }
}

struct Opcode {
    constant: String,
    code: String,
    words: String,
    name: &'static str,
}

#[derive(Template)]
#[template(path = "hwui/Protocol.kt", escape = "none")]
struct Wire {
    protocol: Vec<Constant>,
    ops: Vec<Opcode>,
    objects: Vec<Object>,
    text: Vec<Constant>,
}

fn constants<T: Kotlin>(codes: &Codes<T>) -> Vec<Constant> {
    codes
        .entries
        .iter()
        .map(|&(name, value, doc)| Constant::new(name, value, doc))
        .collect()
}

/// Rust doc lines as one `KDoc` line: intra-doc links become code spans.
fn kdoc(lines: &[&str]) -> String {
    let joined = lines
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = String::with_capacity(joined.len());
    let mut rest = joined.as_str();
    while let Some(at) = rest.find("](") {
        out.push_str(&rest[..at]);
        rest = rest[at + 2..]
            .split_once(')')
            .map_or("", |(_, after)| after);
        out.push(']');
    }
    out.push_str(rest);
    out.replace("[`", "`").replace("`]", "`")
}

/// `CreateNode` as `CREATE_NODE`.
fn screaming(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.char_indices() {
        if ch.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}

/// The framing constants.
fn protocol() -> Vec<Constant> {
    let header_bytes = HEADER_WORDS * 4;
    vec![
        Constant::new(
            "MAGIC",
            MAGIC,
            &["`\"HWUI\"` read as a little-endian word."],
        ),
        Constant::new(
            "HEADER_BYTES",
            header_bytes,
            &["Header bytes: magic, frame sequence, byte length."],
        ),
        Constant::new("NONE", NONE, &["The id that names nothing."]),
        Constant::new("SHAPE_WORDS", SHAPE_WORDS, &["Words of an inline shape."]),
        Constant::new("PAINT_WORDS", PAINT_WORDS, &["Words of an inline paint."]),
        Constant::new(
            "MATRIX_WORDS",
            MATRIX_WORDS,
            &["Words of an Android `Matrix`, in `Matrix.getValues` order."],
        ),
        Constant::new(
            "TRANSFORM_VALUES",
            TRANSFORM_VALUES,
            &["The node-property values of a `SET_TRANSFORM` after its node."],
        ),
    ]
}

/// The text wire's constants.
fn text_wire() -> Vec<Constant> {
    let mut text_wire = vec![Constant::new(
        "SPAN_WORDS",
        wire::SPAN_WORDS,
        &["`int`s per packed style run."],
    )];
    text_wire.extend(constants(&wire::span::CODES));
    text_wire.extend(constants(&wire::run_flags::CODES));
    text_wire.extend(constants(&wire::paragraph::CODES));
    text_wire.extend([
        Constant::new(
            "DEFAULT_FAMILY",
            wire::DEFAULT_FAMILY,
            &["The family index of a run that names none: the platform default."],
        ),
        Constant::new(
            "UNBOUNDED",
            wire::UNBOUNDED,
            &["The wrap width of a request that does not wrap."],
        ),
        Constant::new(
            "NO_LINE_LIMIT",
            wire::NO_LINE_LIMIT,
            &["The line limit of a request without one."],
        ),
        Constant::new(
            "FAMILY_SEPARATOR",
            wire::FAMILY_SEPARATOR,
            &["What joins the names of one family list."],
        ),
        Constant::new(
            "REPLY_HEADER",
            wire::REPLY_HEADER,
            &["Floats before the first line of a reply."],
        ),
        Constant::new(
            "LINE_WORDS",
            wire::LINE_WORDS,
            &["Floats per line of a reply."],
        ),
    ]);
    text_wire
}

impl Wire {
    fn new() -> Self {
        let protocol = protocol();
        let ops = Op::ALL
            .iter()
            .map(|&op| Opcode {
                constant: screaming(op.name()),
                code: format!("0x{:04X}", op.code()),
                words: op
                    .fixed_words()
                    .map_or_else(|| "VARIABLE".to_owned(), |words| words.to_string()),
                name: op.name(),
            })
            .collect();
        let mut mesh = Object::new(&protocol::mesh_interpolation::CODES);
        mesh.constants.extend([
            Constant::new("BAND_PATCHES", MESH_BAND_PATCHES, &["The most patches one mesh op carries: as many as fit a record's 65535 payload words."]),
            Constant::new("PATCH_FLOATS", MESH_PATCH_FLOATS, &["Floats of one patch: corners 00, 10, 01, 11 as `x, y`, then their premultiplied colours."]),
        ]);
        let objects = vec![
            Object::new(&protocol::shape::CODES),
            Object::new(&protocol::paint::CODES),
            Object::new(&protocol::clip::CODES),
            Object::new(&protocol::fill_type::CODES),
            Object::new(&protocol::verb::CODES),
            Object::new(&protocol::tile::CODES),
            Object::new(&protocol::bitmap_format::CODES),
            Object::new(&protocol::color_space::CODES),
            Object::new(&protocol::sampling::CODES),
            Object::new(&protocol::shader::CODES),
            Object::new(&protocol::cap::CODES),
            Object::new(&protocol::join::CODES),
            Object::new(&protocol::glyph_style::CODES),
            Object::new(&protocol::host::CODES),
            mesh,
            Object::new(&protocol::blend::CODES),
        ];
        Self {
            protocol,
            ops,
            objects,
            text: text_wire(),
        }
    }
}

#[test]
fn the_kotlin_wire_is_generated_from_the_rust_definitions() {
    let source = Wire::new().render().expect("the Kotlin wire renders");
    for op in Op::ALL {
        let constant = format!(
            "const val {}: Int = 0x{:04X}\n",
            screaming(op.name()),
            op.code()
        );
        assert!(source.contains(&constant), "{constant}");
        let named = format!("{} -> \"{}\"\n", screaming(op.name()), op.name());
        assert!(source.contains(&named), "{named}");
    }
    assert!(source.contains("const val NONE: Int = -1\n"), "{source}");
    assert!(
        source.contains("SRC_IN -> BlendMode.SRC_IN\n"),
        "blend codes map to the modes of their names: {source}"
    );
    assert!(
        source.contains("const val FAMILY_SEPARATOR: Char = '\\u001f'\n"),
        "{source}"
    );
    assert!(
        source.contains("const val UNBOUNDED: Float = -1.0f\n"),
        "{source}"
    );
    assert!(!source.contains("{{"), "{source}");
    if let Some(dir) = std::env::var_os("HWUI_KOTLIN_DIR") {
        let package = Path::new(&dir).join("dev/waterui/hydrolysis/hwui");
        fs::create_dir_all(&package).expect("the generated package directory is writable");
        fs::write(package.join("Protocol.kt"), source).expect("the generated source is writable");
    }
}

#[test]
fn rust_docs_become_kdoc_lines() {
    assert_eq!(
        kdoc(&[" The run's family list index, or [`DEFAULT_FAMILY`](super::DEFAULT_FAMILY)."]),
        "The run's family list index, or `DEFAULT_FAMILY`."
    );
    assert_eq!(
        kdoc(&[" [`Op::SetClip`] kinds,", " by code."]),
        "`Op::SetClip` kinds, by code."
    );
    assert_eq!(screaming("ReleaseTextLayout"), "RELEASE_TEXT_LAYOUT");
}
