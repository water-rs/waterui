//! The component catalog: what JSX may name, and what each name accepts.
//!
//! A props contract describes one mounted module. This describes the other
//! half of the seam — `WaterUI`'s own component vocabulary — and it is built
//! the same way: from [`TypeSchema`] constants the compiler produced, encoded
//! during const evaluation, and parked in a `#[used] static` the `water` CLI
//! reads back out of the runtime crate's rlib.
//!
//! One table answers every question about the vocabulary: the runtime resolves
//! a JSX tag against it, `installHost` hands JavaScript its modifier names so
//! attribute classification stays a JavaScript-local lookup, `water components
//! --json` prints it, and the `waterui` module's `.d.ts` is generated from it.
//!
//! # Modifiers are catalog-wide
//!
//! A view modifier applies to every view — that is what `ViewExt` being
//! implemented for every `View` means — so the modifiers live beside the
//! components rather than being repeated inside each one. A generator that
//! types one component's attributes takes that component's
//! [`attributes`](ComponentSchema::attributes) and the whole modifier table.

use crate::decode::{DecodeError, Reader};
use crate::encode::{put, put_node, put_str, put_varint};
use crate::format::{FORMAT_VERSION, children, kind};
use crate::owned;
use crate::tree::{StructSchema, TypeSchema};

/// The whole vocabulary: every component JSX may name, and every modifier
/// attribute that may be written on one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogSchema {
    /// The components, in the order the catalog declares them.
    pub components: &'static [ComponentSchema],
    /// The view modifiers, which apply to every component.
    pub modifiers: &'static [ModifierSchema],
}

/// One component: its tag name, its configuration attributes, and what its
/// children are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentSchema {
    /// The JSX tag name — `"VStack"`, `"Toggle"`.
    pub name: &'static str,
    /// One sentence describing the component, for the generated `.d.ts` and
    /// for `water components`.
    pub summary: &'static str,
    /// The configuration attributes: the props struct the host reads a
    /// configuration object with, whose fields are the attributes.
    ///
    /// The schema is that struct's own constant — [`attributes_of`] lifts it
    /// out of its [`TypeSchema`] — so the attribute list and the conversion
    /// that reads the object are the same declaration.
    pub attributes: StructSchema,
    /// What this component's JSX children are.
    pub children: ChildrenSlot,
}

/// One modifier attribute and the shape its value takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifierSchema {
    /// The attribute name — `"padding"`, `"background"`.
    pub name: &'static str,
    /// One sentence describing the modifier.
    pub summary: &'static str,
    /// The value it accepts.
    pub value: &'static TypeSchema,
}

/// What a component's JSX children are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChildrenSlot {
    /// The component takes no children; passing any is an error.
    None,
    /// The children are the component's content views.
    Content,
    /// The children are the component's label, which every control requires at
    /// construction. `label` is the explicit form of the same slot.
    Label,
    /// The children are text content, lifted into one string.
    Text,
}

impl ChildrenSlot {
    /// The byte that encodes this slot.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::None => children::NONE,
            Self::Content => children::CONTENT,
            Self::Label => children::LABEL,
            Self::Text => children::TEXT,
        }
    }

    /// The slot a payload byte denotes, or `None` when the byte is not one.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            children::NONE => Some(Self::None),
            children::CONTENT => Some(Self::Content),
            children::LABEL => Some(Self::Label),
            children::TEXT => Some(Self::Text),
            _ => None,
        }
    }
}

/// Append one component entry.
const fn put_component(buf: &mut [u8], pos: usize, component: &ComponentSchema) -> usize {
    assert!(
        !component.name.is_empty(),
        "a component name must not be empty"
    );
    assert!(
        !component.summary.is_empty(),
        "a component carries a one-sentence summary"
    );
    let pos = put_str(buf, pos, component.name);
    let pos = put_str(buf, pos, component.summary);
    let pos = put(buf, pos, component.children.tag());
    // The attributes node sits one level below the catalog root, exactly as a
    // props contract's root node does.
    put_node(buf, pos, &TypeSchema::Struct(component.attributes), 1)
}

/// Append one modifier entry.
const fn put_modifier(buf: &mut [u8], pos: usize, modifier: &ModifierSchema) -> usize {
    assert!(
        !modifier.name.is_empty(),
        "a modifier name must not be empty"
    );
    assert!(
        !modifier.summary.is_empty(),
        "a modifier carries a one-sentence summary"
    );
    let pos = put_str(buf, pos, modifier.name);
    let pos = put_str(buf, pos, modifier.summary);
    put_node(buf, pos, modifier.value, 1)
}

/// Write the whole catalog into `buf`, returning the length.
const fn put_catalog(buf: &mut [u8], catalog: &CatalogSchema) -> usize {
    assert!(
        !catalog.components.is_empty(),
        "a catalog with no components would let JSX name nothing"
    );
    let pos = put(buf, 0, FORMAT_VERSION);
    let mut pos = put(buf, pos, kind::CATALOG);
    pos = put_varint(buf, pos, catalog.components.len());
    let mut index = 0;
    while index < catalog.components.len() {
        pos = put_component(buf, pos, &catalog.components[index]);
        index += 1;
    }
    pos = put_varint(buf, pos, catalog.modifiers.len());
    let mut index = 0;
    while index < catalog.modifiers.len() {
        pos = put_modifier(buf, pos, &catalog.modifiers[index]);
        index += 1;
    }
    pos
}

/// Length in bytes of `catalog`'s encoded payload, not counting the NUL
/// terminator the artifact static appends.
///
/// # Panics
/// Fails const evaluation on the same violations [`encode_catalog`] rejects.
#[must_use]
pub const fn catalog_encoded_len(catalog: &CatalogSchema) -> usize {
    let mut probe: [u8; 0] = [];
    put_catalog(&mut probe, catalog)
}

/// Encode `catalog` into a NUL-terminated array.
///
/// `N` must be `catalog_encoded_len(catalog) + 1`, for the same reason
/// [`encode`](crate::encode) requires it of a props contract: the CLI finds a
/// static's end at its first NUL, because a Mach-O symbol carries no size.
///
/// # Panics
/// Fails const evaluation when `N` is not `catalog_encoded_len(catalog) + 1`,
/// when the catalog is empty, when a name or summary is empty, or when a
/// component's attributes are not an object type.
#[must_use]
pub const fn encode_catalog<const N: usize>(catalog: &CatalogSchema) -> [u8; N] {
    let mut encoded = [0_u8; N];
    let end = put_catalog(&mut encoded, catalog);
    assert!(
        end + 1 == N,
        "encode_catalog::<N> requires N == catalog_encoded_len(catalog) + 1"
    );
    encoded
}

/// An owned [`CatalogSchema`], recovered from an artifact.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    /// The components, in catalog order.
    pub components: Vec<Component>,
    /// The view modifiers, which apply to every component.
    pub modifiers: Vec<Modifier>,
}

/// An owned [`ComponentSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Component {
    /// The JSX tag name.
    pub name: String,
    /// One sentence describing the component.
    pub summary: String,
    /// The configuration attributes, in declaration order.
    pub attributes: Vec<owned::Field>,
    /// What this component's children are.
    pub children: ChildrenSlot,
}

/// An owned [`ModifierSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Modifier {
    /// The attribute name.
    pub name: String,
    /// One sentence describing the modifier.
    pub summary: String,
    /// The value it accepts.
    pub value: owned::Schema,
}

impl From<&CatalogSchema> for Catalog {
    fn from(catalog: &CatalogSchema) -> Self {
        Self {
            components: catalog.components.iter().map(Component::from).collect(),
            modifiers: catalog.modifiers.iter().map(Modifier::from).collect(),
        }
    }
}

impl From<&ComponentSchema> for Component {
    fn from(component: &ComponentSchema) -> Self {
        Self {
            name: component.name.to_owned(),
            summary: component.summary.to_owned(),
            attributes: component
                .attributes
                .fields
                .iter()
                .map(owned::Field::from)
                .collect(),
            children: component.children,
        }
    }
}

/// The object type a props struct's schema is, for a catalog entry's
/// [`attributes`](ComponentSchema::attributes).
///
/// A component's attributes are declared as a struct deriving
/// [`TsType`](crate::TsType), and this is how that constant becomes a catalog
/// entry: `attributes: attributes_of(&<ToggleAttributes as TsType>::SCHEMA)`.
///
/// # Panics
/// Fails const evaluation when the schema is not a struct — a props struct's
/// always is, so the panic names a type that is not one.
#[must_use]
pub const fn attributes_of(schema: &'static TypeSchema) -> StructSchema {
    match schema {
        TypeSchema::Struct(schema) => *schema,
        _ => panic!("a component's configuration attributes are declared as a struct"),
    }
}

impl From<&ModifierSchema> for Modifier {
    fn from(modifier: &ModifierSchema) -> Self {
        Self {
            name: modifier.name.to_owned(),
            summary: modifier.summary.to_owned(),
            value: owned::Schema::from(modifier.value),
        }
    }
}

/// Decode a catalog payload into an owned catalog.
///
/// The payload is the byte range between the version byte and the NUL
/// terminator, exactly as [`decode`](crate::decode) takes a props contract.
///
/// # Errors
/// Returns a [`DecodeError`] for an unknown version, a props contract handed
/// to the catalog decoder, a truncated or malformed payload, a component whose
/// attributes are not an object type, or bytes left over after the table.
pub fn decode_catalog(payload: &[u8]) -> Result<Catalog, DecodeError> {
    if payload.is_empty() {
        return Err(DecodeError::Empty);
    }
    let mut reader = Reader {
        bytes: payload,
        pos: 0,
        depth: 0,
    };
    let version = reader.byte()?;
    if version != FORMAT_VERSION {
        return Err(DecodeError::Version {
            found: version,
            expected: FORMAT_VERSION,
        });
    }
    if reader.byte()? != kind::CATALOG {
        return Err(DecodeError::NotACatalog);
    }

    let count = reader.length()?;
    if count == 0 {
        return Err(DecodeError::EmptyCatalog);
    }
    let mut components = Vec::new();
    for _ in 0..count {
        components.push(read_component(&mut reader)?);
    }

    let count = reader.length()?;
    let mut modifiers = Vec::new();
    for _ in 0..count {
        modifiers.push(read_modifier(&mut reader)?);
    }

    let extra = payload.len() - reader.pos;
    if extra > 0 {
        return Err(DecodeError::Trailing { extra });
    }
    Ok(Catalog {
        components,
        modifiers,
    })
}

/// Read one component entry.
fn read_component(reader: &mut Reader<'_>) -> Result<Component, DecodeError> {
    let name = named(reader, "component")?;
    let summary = named(reader, "summary")?;
    let offset = reader.pos;
    let tag = reader.byte()?;
    let children = ChildrenSlot::from_tag(tag).ok_or(DecodeError::UnknownTag {
        kind: "children",
        tag,
        offset,
    })?;
    // Each entry's attributes are a fresh tree, so the node budget is the
    // entry's, not the whole catalog's.
    reader.depth = 0;
    let attributes = match reader.node()? {
        owned::Schema::Struct(attributes) => attributes.fields,
        other => {
            return Err(DecodeError::AttributesNotAStruct {
                component: name,
                found: kind_of(&other),
            });
        }
    };
    Ok(Component {
        name,
        summary,
        attributes,
        children,
    })
}

/// Read one modifier entry.
fn read_modifier(reader: &mut Reader<'_>) -> Result<Modifier, DecodeError> {
    let name = named(reader, "modifier")?;
    let summary = named(reader, "summary")?;
    reader.depth = 0;
    let value = reader.node()?;
    Ok(Modifier {
        name,
        summary,
        value,
    })
}

/// Read one name, which the encoder guarantees is not empty.
fn named(reader: &mut Reader<'_>, what: &'static str) -> Result<String, DecodeError> {
    let offset = reader.pos;
    let name = reader.string()?;
    if name.is_empty() {
        return Err(DecodeError::EmptyName { kind: what, offset });
    }
    Ok(name)
}

/// What a node is, for the error a non-struct attribute list raises.
const fn kind_of(schema: &owned::Schema) -> &'static str {
    match schema {
        owned::Schema::Unit => "void",
        owned::Schema::Bool => "boolean",
        owned::Schema::Number(_) => "a number",
        owned::Schema::String => "string",
        owned::Schema::Option(_) => "an optional",
        owned::Schema::List(_) => "a list",
        owned::Schema::Array { .. } => "a tuple",
        owned::Schema::Map { .. } => "a record",
        owned::Schema::Signal(_) => "a signal",
        owned::Schema::Accessor(_) => "an accessor",
        owned::Schema::View => "a view",
        owned::Schema::Callback(_) => "a callback",
        owned::Schema::Union(_) => "a union",
        owned::Schema::Struct(_) => "an object",
        owned::Schema::Enum(_) => "an enum",
    }
}
