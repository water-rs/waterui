//! The owned mirror of the const tree.
//!
//! [`crate::TypeSchema`] is built from `&'static` data because it has to be a
//! constant. A schema recovered from a compiled artifact owns its data
//! instead, and is `serde`-serializable so the `water` CLI can print it or
//! hand it to a `.d.ts` generator.

use crate::tree::{
    EnumRepresentation, EnumSchema, FieldSchema, NumberKind, StructSchema, TypeSchema,
    VariantPayload, VariantSchema,
};

/// An owned [`TypeSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Schema {
    /// See [`TypeSchema::Unit`].
    Unit,
    /// See [`TypeSchema::Bool`].
    Bool,
    /// See [`TypeSchema::Number`].
    Number(NumberKind),
    /// See [`TypeSchema::String`].
    String,
    /// See [`TypeSchema::Option`].
    Option(Box<Self>),
    /// See [`TypeSchema::List`].
    List(Box<Self>),
    /// See [`TypeSchema::Array`].
    Array {
        /// The element type.
        item: Box<Self>,
        /// How many elements the array carries.
        len: usize,
    },
    /// See [`TypeSchema::Map`].
    Map {
        /// The key type.
        key: Box<Self>,
        /// The value type.
        value: Box<Self>,
    },
    /// See [`TypeSchema::Signal`].
    Signal(Box<Self>),
    /// See [`TypeSchema::Accessor`].
    Accessor(Box<Self>),
    /// See [`TypeSchema::View`].
    View,
    /// See [`TypeSchema::ViewBuilder`].
    ViewBuilder,
    /// See [`TypeSchema::Callback`].
    Callback(Vec<Self>),
    /// See [`TypeSchema::Union`].
    Union(Vec<Self>),
    /// See [`TypeSchema::Struct`].
    Struct(Struct),
    /// See [`TypeSchema::Enum`].
    Enum(Enum),
}

/// An owned [`StructSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Struct {
    /// The type name.
    pub name: String,
    /// The fields in declaration order.
    pub fields: Vec<Field>,
}

impl Struct {
    /// Creates a Struct from its parts.
    #[must_use]
    pub const fn new(name: String, fields: Vec<Field>) -> Self {
        Self { name, fields }
    }

    /// The type name.
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The fields in declaration order.
    #[must_use]
    pub const fn fields(&self) -> &[Field] {
        self.fields.as_slice()
    }

    /// Takes the fields in declaration order.
    #[must_use]
    pub fn into_fields(self) -> Vec<Field> {
        self.fields
    }
}

/// An owned [`FieldSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Field {
    /// The field name.
    pub name: String,
    /// The field type.
    pub ty: Schema,
}

impl Field {
    /// Creates a Field from its parts.
    #[must_use]
    pub const fn new(name: String, ty: Schema) -> Self {
        Self { name, ty }
    }

    /// The field name.
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The field type.
    #[must_use]
    pub const fn ty(&self) -> &Schema {
        &self.ty
    }
}

/// An owned [`EnumSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Enum {
    /// The type name.
    pub name: String,
    /// How the variants are laid out on the TypeScript side.
    pub representation: Representation,
    /// The variants in declaration order.
    pub variants: Vec<Variant>,
}

impl Enum {
    /// Creates a Enum from its parts.
    #[must_use]
    pub const fn new(name: String, representation: Representation, variants: Vec<Variant>) -> Self {
        Self {
            name,
            representation,
            variants,
        }
    }

    /// The type name.
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// How the variants are laid out on the TypeScript side.
    #[must_use]
    pub const fn representation(&self) -> &Representation {
        &self.representation
    }

    /// The variants in declaration order.
    #[must_use]
    pub const fn variants(&self) -> &[Variant] {
        self.variants.as_slice()
    }
}

/// An owned [`EnumRepresentation`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Representation {
    /// See [`EnumRepresentation::StringUnion`].
    StringUnion,
    /// See [`EnumRepresentation::Tagged`].
    Tagged {
        /// The property holding the variant name.
        tag: String,
        /// The property holding the variant payload.
        content: String,
    },
}

/// An owned [`VariantSchema`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Variant {
    /// The variant name.
    pub name: String,
    /// What the variant carries.
    pub payload: Payload,
}

impl Variant {
    /// Creates a Variant from its parts.
    #[must_use]
    pub const fn new(name: String, payload: Payload) -> Self {
        Self { name, payload }
    }

    /// The variant name.
    #[must_use]
    pub const fn name(&self) -> &str {
        self.name.as_str()
    }

    /// What the variant carries.
    #[must_use]
    pub const fn payload(&self) -> &Payload {
        &self.payload
    }
}

/// An owned [`VariantPayload`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Payload {
    /// See [`VariantPayload::Unit`].
    Unit,
    /// See [`VariantPayload::Tuple`].
    Tuple(Vec<Schema>),
    /// See [`VariantPayload::Struct`].
    Struct(Vec<Field>),
}

impl From<&TypeSchema> for Schema {
    fn from(schema: &TypeSchema) -> Self {
        match schema {
            TypeSchema::Unit => Self::Unit,
            TypeSchema::Bool => Self::Bool,
            TypeSchema::Number(kind) => Self::Number(*kind),
            TypeSchema::String => Self::String,
            TypeSchema::Option(inner) => Self::Option(Box::new(Self::from(*inner))),
            TypeSchema::List(inner) => Self::List(Box::new(Self::from(*inner))),
            TypeSchema::Array { item, len } => Self::Array {
                item: Box::new(Self::from(*item)),
                len: *len,
            },
            TypeSchema::Map { key, value } => Self::Map {
                key: Box::new(Self::from(*key)),
                value: Box::new(Self::from(*value)),
            },
            TypeSchema::Signal(inner) => Self::Signal(Box::new(Self::from(*inner))),
            TypeSchema::Accessor(inner) => Self::Accessor(Box::new(Self::from(*inner))),
            TypeSchema::Union(members) => Self::Union(members.iter().map(Self::from).collect()),
            TypeSchema::View => Self::View,
            TypeSchema::ViewBuilder => Self::ViewBuilder,
            TypeSchema::Callback(arguments) => {
                Self::Callback(arguments.iter().map(Self::from).collect())
            }
            TypeSchema::Struct(schema) => Self::Struct(Struct::from(schema)),
            TypeSchema::Enum(schema) => Self::Enum(Enum::from(schema)),
        }
    }
}

impl From<&StructSchema> for Struct {
    fn from(schema: &StructSchema) -> Self {
        Self {
            name: schema.name().to_owned(),
            fields: schema.fields().iter().map(Field::from).collect(),
        }
    }
}

impl From<&FieldSchema> for Field {
    fn from(field: &FieldSchema) -> Self {
        Self {
            name: field.name().to_owned(),
            ty: Schema::from(field.ty()),
        }
    }
}

impl From<&EnumSchema> for Enum {
    fn from(schema: &EnumSchema) -> Self {
        Self {
            name: schema.name().to_owned(),
            representation: Representation::from(&schema.representation()),
            variants: schema.variants().iter().map(Variant::from).collect(),
        }
    }
}

impl From<&EnumRepresentation> for Representation {
    fn from(representation: &EnumRepresentation) -> Self {
        match representation {
            EnumRepresentation::StringUnion => Self::StringUnion,
            EnumRepresentation::Tagged { tag, content } => Self::Tagged {
                tag: (*tag).to_owned(),
                content: (*content).to_owned(),
            },
        }
    }
}

impl From<&VariantSchema> for Variant {
    fn from(variant: &VariantSchema) -> Self {
        Self {
            name: variant.name().to_owned(),
            payload: Payload::from(&variant.payload()),
        }
    }
}

impl From<&VariantPayload> for Payload {
    fn from(payload: &VariantPayload) -> Self {
        match payload {
            VariantPayload::Unit => Self::Unit,
            VariantPayload::Tuple(nodes) => Self::Tuple(nodes.iter().map(Schema::from).collect()),
            VariantPayload::Struct(fields) => {
                Self::Struct(fields.iter().map(Field::from).collect())
            }
        }
    }
}
