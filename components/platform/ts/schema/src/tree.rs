//! The const schema tree.
//!
//! Every node is built from `&'static` data so a whole tree is a constant
//! expression: a type's schema is `<T as TsType>::SCHEMA`, and a nested field
//! composes by naming the field type's own constant. Aliases, generic
//! parameters and `cfg`s are therefore resolved by the compiler rather than by
//! matching on how a type happens to be spelled.

use core::fmt;

/// The Rust numeric type a [`TypeSchema::Number`] node describes.
///
/// TypeScript has one `number` (an IEEE-754 double) and one `bigint`, so the
/// projection is lossy in one direction only: every Rust integer that fits in
/// 53 bits of mantissa becomes `number`, and the 64-bit and pointer-sized
/// integers become `bigint`. That is the same 2^53 boundary the web view
/// bridge draws, and for the same reason — a `u64` round-tripped through a
/// double loses its low bits silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NumberKind {
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `i64`.
    I64,
    /// `u64`.
    U64,
    /// `isize`.
    Isize,
    /// `usize`.
    Usize,
}

impl NumberKind {
    /// Whether TypeScript sees this kind as `bigint` rather than `number`.
    #[must_use]
    pub const fn is_bigint(self) -> bool {
        matches!(self, Self::I64 | Self::U64 | Self::Isize | Self::Usize)
    }

    /// The TypeScript type name this kind projects to.
    #[must_use]
    pub const fn ts_name(self) -> &'static str {
        if self.is_bigint() { "bigint" } else { "number" }
    }

    /// The byte that follows the `Number` node tag in an encoded payload.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::F32 => 1,
            Self::F64 => 2,
            Self::I8 => 3,
            Self::I16 => 4,
            Self::I32 => 5,
            Self::U8 => 6,
            Self::U16 => 7,
            Self::U32 => 8,
            Self::I64 => 9,
            Self::U64 => 10,
            Self::Isize => 11,
            Self::Usize => 12,
        }
    }

    /// The kind a payload byte denotes, or `None` when the byte is not one.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::F32),
            2 => Some(Self::F64),
            3 => Some(Self::I8),
            4 => Some(Self::I16),
            5 => Some(Self::I32),
            6 => Some(Self::U8),
            7 => Some(Self::U16),
            8 => Some(Self::U32),
            9 => Some(Self::I64),
            10 => Some(Self::U64),
            11 => Some(Self::Isize),
            12 => Some(Self::Usize),
            _ => None,
        }
    }
}

/// The TypeScript-facing shape of a Rust type.
///
/// Nodes reference their children through `&'static Self`, which a const
/// initializer produces by borrowing the child type's own `SCHEMA` constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeSchema {
    /// `()`, projected as `void`.
    Unit,
    /// `bool`, projected as `boolean`.
    Bool,
    /// A Rust numeric type, projected as `number` or `bigint`.
    Number(NumberKind),
    /// A Rust string type, projected as `string`.
    String,
    /// `Option<T>`, projected as `T | null`.
    Option(&'static Self),
    /// A homogeneous sequence of any length — `Vec<T>` or a slice —
    /// projected as `T[]`.
    List(&'static Self),
    /// A fixed-length array — `[T; N]` — projected as the tuple type
    /// `[T, T, …]` with one slot per element.
    ///
    /// The length is part of the type because it is part of the contract: the
    /// conversion refuses an array of any other length, and `T[]` would
    /// promise TypeScript something the seam does not accept.
    Array {
        /// The element type.
        item: &'static Self,
        /// How many elements the array carries, at most
        /// [`MAX_ARRAY_LEN`](crate::MAX_ARRAY_LEN).
        len: usize,
    },
    /// A string-keyed map, projected as `Record<K, V>`.
    Map {
        /// The key type; a string type by construction, see
        /// [`TsMapKey`](crate::TsMapKey).
        key: &'static Self,
        /// The value type.
        value: &'static Self,
    },
    /// A two-way reactive cell — `Binding<T>` — projected as `Signal<T>`.
    Signal(&'static Self),
    /// A read-only reactive value — `Computed<T>` — projected as `Accessor<T>`.
    Accessor(&'static Self),
    /// An opaque handle to a Rust-composed view subtree, projected as `View`.
    View,
    /// A function that builds a view each time it is called, projected as
    /// `() => JSX.Element`.
    ///
    /// A [`View`](Self::View) handle crosses once and is taken once: it is a
    /// subtree that already exists. A destination or a page is not that — a
    /// `ViewBuilder` may be built again at any time, and each build needs a
    /// fresh subtree — so what crosses is the function, called per build.
    ViewBuilder,
    /// A callback the TypeScript side invokes, projected as
    /// `(…) => void`. The slice holds the argument types in order.
    Callback(&'static [Self]),
    /// A choice of shapes, projected as the union `A | B | …`.
    ///
    /// No Rust type projects to a union — a field has one type — so no derive
    /// produces this node. It exists for the component catalog, where an
    /// attribute genuinely accepts several shapes that the Rust side converts
    /// from: `padding` takes `true`, a number or an edge-insets object, which
    /// is a union in TypeScript exactly as it is a set of `From` impls in
    /// Rust. A type that declares one writes its own [`TsType`](crate::TsType)
    /// impl beside the conversion that reads the same shapes.
    Union(&'static [Self]),
    /// A struct with named fields, projected as an object type.
    Struct(StructSchema),
    /// An enum, projected according to its [`EnumRepresentation`].
    Enum(EnumSchema),
}

/// A struct with named fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructSchema {
    /// The Rust type name, which is also the generated TypeScript type name.
    pub name: &'static str,
    /// The fields in declaration order.
    pub fields: &'static [FieldSchema],
}

/// One named field of a struct or a struct-shaped enum variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldSchema {
    /// The field name, which is also the generated property name.
    pub name: &'static str,
    /// The field type.
    pub ty: TypeSchema,
}

/// An enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumSchema {
    /// The Rust type name, which is also the generated TypeScript type name.
    pub name: &'static str,
    /// How the variants are laid out on the TypeScript side.
    pub representation: EnumRepresentation,
    /// The variants in declaration order.
    pub variants: &'static [VariantSchema],
}

/// How an enum's variants are laid out on the TypeScript side.
///
/// The representation is recorded in the schema, not inferred from `serde`
/// attributes at conversion time: the runtime converter follows the schema, so
/// the only place the layout is decided is here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumRepresentation {
    /// Every variant is a unit variant, so the enum is a union of string
    /// literals: `"Idle" | "Running"`.
    StringUnion,
    /// At least one variant carries data, so every variant is an object
    /// carrying its name under `tag` and, when it has a payload, its data
    /// under `content`. Adjacent tagging is the one layout that works for
    /// unit, tuple and struct payloads alike.
    Tagged {
        /// The property holding the variant name.
        tag: &'static str,
        /// The property holding the variant payload.
        content: &'static str,
    },
}

impl EnumRepresentation {
    /// The property names a derived enum uses when it carries data.
    pub const DEFAULT_TAGGED: Self = Self::Tagged {
        tag: "type",
        content: "value",
    };
}

/// One variant of an enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VariantSchema {
    /// The variant name, which is also its tag value.
    pub name: &'static str,
    /// What the variant carries.
    pub payload: VariantPayload,
}

/// What an enum variant carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantPayload {
    /// Nothing.
    Unit,
    /// Positional fields, projected as a tuple type.
    Tuple(&'static [TypeSchema]),
    /// Named fields, projected as an object type.
    Struct(&'static [FieldSchema]),
}

impl TypeSchema {
    /// Whether this node renders as a union, and therefore needs parentheses
    /// where TypeScript's `[]` suffix would otherwise bind tighter than `|`.
    ///
    /// An `Option<T>` renders as `T | null`, which is a union however it is
    /// spelled in Rust.
    const fn is_union(&self) -> bool {
        matches!(self, Self::Union(_) | Self::Option(_))
    }
}

impl fmt::Display for TypeSchema {
    /// Renders the node as the TypeScript type expression that refers to it.
    ///
    /// Structs and enums render as their name — the expression a `.d.ts`
    /// writes at a use site — rather than expanding their body, which keeps
    /// the output finite for a recursive schema.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("void"),
            Self::Bool => f.write_str("boolean"),
            Self::Number(kind) => f.write_str(kind.ts_name()),
            Self::String => f.write_str("string"),
            Self::Option(inner) => write!(f, "{inner} | null"),
            // `T[]` binds tighter than `|`, so an element type that is itself
            // a union or an option needs parentheses: without them
            // `List(Union(Bool, Number))` reads as `boolean | number[]`,
            // which is a different type — a boolean or an array of numbers.
            Self::List(inner) if inner.is_union() => write!(f, "({inner})[]"),
            Self::List(inner) => write!(f, "{inner}[]"),
            Self::Array { item, len } => {
                f.write_str("[")?;
                for index in 0..*len {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
            Self::Map { key, value } => write!(f, "Record<{key}, {value}>"),
            Self::Signal(inner) => write!(f, "Signal<{inner}>"),
            Self::Accessor(inner) => write!(f, "Accessor<{inner}>"),
            Self::Union(members) => {
                for (index, member) in members.iter().enumerate() {
                    if index > 0 {
                        f.write_str(" | ")?;
                    }
                    write!(f, "{member}")?;
                }
                Ok(())
            }
            Self::View => f.write_str("View"),
            Self::ViewBuilder => f.write_str("() => JSX.Element"),
            Self::Callback(arguments) => {
                f.write_str("(")?;
                for (index, argument) in arguments.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "arg{index}: {argument}")?;
                }
                f.write_str(") => void")
            }
            Self::Struct(schema) => f.write_str(schema.name),
            Self::Enum(schema) => f.write_str(schema.name),
        }
    }
}
