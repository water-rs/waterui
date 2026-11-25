//! Schema-backed `#[js_api]` trait and TypeScript declaration generator.
//!
//! `#[js_api]` records every exposed state field and handler as a
//! [`JsApiSchema`] built in `const` from [`TsType::SCHEMA`] and
//! [`JsReplySchema::SCHEMA`]. Because the compiler resolves type aliases,
//! re-exports and user-defined `#[derive(TsType)]` types before const
//! evaluation, the generated `.d.ts` carries exact TypeScript shapes without
//! ever matching on token spellings.

use std::collections::BTreeSet;
use std::fmt::{Display, Write as _};
use std::rc::Rc;

use serde::Serialize;
use suiteki::Str;
use waterui_ts_schema::{
    EnumRepresentation, EnumSchema, FieldSchema, StructSchema, TsType, TypeSchema, VariantPayload,
};

use crate::{Bytes, IntoJsReply, Json, WebViewOpen};

/// What a `#[js_api]` handler resolves its page-side promise to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsApiReturn {
    /// A JSON-compatible value (or `()` for `void`) described by its [`TypeSchema`].
    Schema(TypeSchema),
    /// Raw binary data (`Bytes` or `Vec<u8>`), arriving on the page as `Uint8Array`.
    Bytes,
}

/// Declares the TypeScript return shape of a `#[js_api]` handler.
///
/// Implemented for every [`IntoJsReply`] shape whose payload has a statically
/// known TypeScript projection: `()`, `String`, `Str`, `Bytes`, `Vec<u8>`,
/// `Json<T>` where `T: TsType`, and `Result<T, E>` of any of those.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no TypeScript reply schema for `#[js_api]`",
    label = "cannot derive a TypeScript return type for `{Self}`",
    note = "return `()`, `String`, `Str`, `Bytes`, `Vec<u8>`, `Json<T>` where `T: TsType`, \
            or `Result<T, E>` of one of those"
)]
pub trait JsReplySchema: IntoJsReply {
    /// The TypeScript shape the page's promise resolves to.
    const SCHEMA: JsApiReturn;
}

impl JsReplySchema for () {
    const SCHEMA: JsApiReturn = JsApiReturn::Schema(TypeSchema::Unit);
}

impl JsReplySchema for String {
    const SCHEMA: JsApiReturn = JsApiReturn::Schema(TypeSchema::String);
}

impl JsReplySchema for Str {
    const SCHEMA: JsApiReturn = JsApiReturn::Schema(TypeSchema::String);
}

impl JsReplySchema for Vec<u8> {
    const SCHEMA: JsApiReturn = JsApiReturn::Bytes;
}

impl JsReplySchema for Bytes {
    const SCHEMA: JsApiReturn = JsApiReturn::Bytes;
}

impl<T: Serialize + TsType> JsReplySchema for Json<T> {
    const SCHEMA: JsApiReturn = JsApiReturn::Schema(T::SCHEMA);
}

impl<T: JsReplySchema, E: Display> JsReplySchema for Result<T, E> {
    const SCHEMA: JsApiReturn = T::SCHEMA;
}

/// One mirrored state entry on a `#[js_api]` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsApiStateField {
    name: &'static str,
    schema: TypeSchema,
}

impl JsApiStateField {
    /// Creates a state field descriptor from its exposed key and [`TsType::SCHEMA`].
    #[must_use]
    pub const fn new(name: &'static str, schema: TypeSchema) -> Self {
        Self { name, schema }
    }

    /// The property name exposed under `waterui.state`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The schema of the mirrored reactive cell (`Signal` for writable
    /// `Binding<T>`, `Accessor` for read-only `Computed<T>`).
    #[must_use]
    pub const fn schema(&self) -> &TypeSchema {
        &self.schema
    }

    /// Whether the page may assign to this field (`Binding<T>`).
    #[must_use]
    pub const fn is_writable(&self) -> bool {
        matches!(self.schema, TypeSchema::Signal(_))
    }
}

/// One named parameter of a `#[js_api]` handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsApiParam {
    name: &'static str,
    schema: TypeSchema,
}

impl JsApiParam {
    /// Creates a parameter descriptor from its argument name and [`TsType::SCHEMA`].
    #[must_use]
    pub const fn new(name: &'static str, schema: TypeSchema) -> Self {
        Self { name, schema }
    }

    /// The parameter property name in the handler's argument object.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The parameter's [`TypeSchema`].
    #[must_use]
    pub const fn schema(&self) -> &TypeSchema {
        &self.schema
    }
}

/// One invokable handler on a `#[js_api]` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsApiMethod {
    name: &'static str,
    params: &'static [JsApiParam],
    returns: JsApiReturn,
}

impl JsApiMethod {
    /// Creates a handler descriptor from its exposed name, parameters and return schema.
    #[must_use]
    pub const fn new(
        name: &'static str,
        params: &'static [JsApiParam],
        returns: JsApiReturn,
    ) -> Self {
        Self {
            name,
            params,
            returns,
        }
    }

    /// The handler name passed to `waterui.invoke`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The named parameters in signature order.
    #[must_use]
    pub const fn params(&self) -> &'static [JsApiParam] {
        self.params
    }

    /// The return schema the page's promise resolves to.
    #[must_use]
    pub const fn returns(&self) -> JsApiReturn {
        self.returns
    }
}

/// The complete schema of a `#[js_api]` surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsApiSchema {
    state: &'static [JsApiStateField],
    methods: &'static [JsApiMethod],
}

impl JsApiSchema {
    /// Creates a surface schema from its mirrored state fields and invokable handlers.
    #[must_use]
    pub const fn new(state: &'static [JsApiStateField], methods: &'static [JsApiMethod]) -> Self {
        Self { state, methods }
    }

    /// The mirrored state fields in declaration order.
    #[must_use]
    pub const fn state(&self) -> &'static [JsApiStateField] {
        self.state
    }

    /// The invokable handlers in declaration order.
    #[must_use]
    pub const fn methods(&self) -> &'static [JsApiMethod] {
        self.methods
    }
}

/// An object whose methods and state are exposed to the page.
///
/// Implemented by `#[js_api]`; there is no reason to write one by hand, and
/// nothing stops you calling [`WebViewOpen::handler`] and
/// [`WebViewOpen::expose`] directly instead — the macro emits exactly those
/// calls, so the two paths stay interchangeable.
///
/// How a method is exposed follows from its shape:
///
/// | Shape | Exposed as | Page writes it back? |
/// | --- | --- | --- |
/// | `async fn(&self, ..) -> R` | a handler the page awaits | — |
/// | `fn(&self) -> Binding<T>` | mirrored state | yes |
/// | `fn(&self) -> Computed<T>` | mirrored state | no |
/// | `#[js(skip)]` anything | not exposed | — |
///
/// Every exposed parameter, state value and `Json<T>` return type must
/// implement [`TsType`]:
///
/// ```
/// use waterui::webview::WebView;
/// use waterui::{Binding, js_api};
///
/// struct Counter {
///     count: Binding<u32>,
/// }
///
/// #[js_api(namespace = "counter")]
/// impl Counter {
///     /// The page reads and assigns `counter.count`.
///     fn count(&self) -> Binding<u32> {
///         self.count.clone()
///     }
///
///     /// The page calls `await counter.add({ by: 2 })`.
///     async fn add(&self, by: u32) {
///         *self.count.get_mut() += by;
///     }
/// }
///
/// let view = WebView::open("https://app.waterui.dev").serve(Counter {
///     count: Binding::new(0),
/// });
/// ```
///
/// A synchronous method that takes arguments is neither of those things — it
/// cannot be state, and a handler must be able to await a reply — so it is
/// rejected rather than silently ignored:
///
/// ```compile_fail
/// use waterui::js_api;
///
/// struct Api;
///
/// #[js_api]
/// impl Api {
///     fn add(&self, by: u32) -> u32 {
///         by + 1
///     }
/// }
/// ```
///
/// An unknown or misspelled `#[js(...)]` attribute is rejected at compile time:
///
/// ```compile_fail
/// use waterui::js_api;
///
/// struct Api;
///
/// #[js_api]
/// impl Api {
///     #[js(rneame = "x")]
///     async fn ping(&self) {}
/// }
/// ```
///
/// A parameter or `Json<T>` payload without [`TsType`] fails to compile rather
/// than silently degrading to `unknown`:
///
/// ```compile_fail
/// use waterui::js_api;
/// use waterui::webview::{Json, serde};
///
/// #[derive(serde::Serialize)]
/// #[serde(crate = "waterui::webview::serde")]
/// struct Opaque {
///     text: String,
/// }
///
/// struct Api;
///
/// #[js_api]
/// impl Api {
///     async fn greet(&self) -> Json<Opaque> {
///         Json(Opaque {
///             text: String::new(),
///         })
///     }
/// }
/// ```
pub trait JsApi: Sized + 'static {
    /// Registers everything this object exposes.
    fn register(api: Rc<Self>, builder: WebViewOpen) -> WebViewOpen;

    /// The compiled schema of every mirrored state field and handler this
    /// surface exposes.
    fn schema() -> &'static JsApiSchema;

    /// The members of this surface, as TypeScript.
    ///
    /// One line per exposed method or field, indented to sit inside an
    /// interface. [`typescript_declarations`] wraps these — together with any
    /// named `interface` and `type` declarations referenced by [`Self::schema`]
    /// — into a file a page's tooling can consume.
    #[must_use]
    fn typescript() -> String {
        render_members(Self::schema())
    }
}

/// A `.d.ts` describing what `A` exposes, for a page's TypeScript to consume.
///
/// Write it next to the page's sources from a test, the way `ts-rs` does, so a
/// surface that changes without the declarations being regenerated fails on the
/// page rather than in production:
///
/// ```ignore
/// #[test]
/// fn declarations_are_current() {
///     let generated = waterui::webview::typescript_declarations::<PageApi>();
///     let checked_in = std::fs::read_to_string("web/waterui.d.ts").unwrap();
///     assert_eq!(generated, checked_in, "run with UPDATE=1 to refresh");
/// }
/// ```
///
/// Every parameter, state value and `Json<T>` reply type is projected from its
/// [`TsType::SCHEMA`], so type aliases resolve to their underlying TypeScript
/// shapes and `#[derive(TsType)]` structs and enums are declared alongside
/// `WaterUiApi` with their real fields and variants.
#[must_use]
pub fn typescript_declarations<A: JsApi>() -> String {
    let schema = A::schema();
    let mut output = String::from("// Generated from a #[js_api] surface. Do not edit.\n\n");

    let named = collect_named_types(schema);
    for decl in named {
        match decl {
            NamedType::Struct(struct_schema) => render_struct_decl(&mut output, struct_schema),
            NamedType::Enum(enum_schema) => render_enum_decl(&mut output, enum_schema),
        }
        output.push('\n');
    }

    let members = render_members(schema);
    let _ = writeln!(output, "interface WaterUiApi {{");
    let _ = writeln!(output, "{members}");
    let _ = writeln!(output, "  readonly ready: Promise<void>;");
    let _ = writeln!(
        output,
        "  watch(key: string, callback: (value: unknown) => void): () => void;"
    );
    let _ = writeln!(output, "}}");
    output.push('\n');
    let _ = writeln!(output, "declare const waterui: WaterUiApi;");
    output
}

enum NamedType {
    Struct(StructSchema),
    Enum(EnumSchema),
}

fn collect_named_types(schema: &JsApiSchema) -> Vec<NamedType> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for field in schema.state() {
        collect_from_schema(field.schema(), &mut seen, &mut out);
    }
    for method in schema.methods() {
        for param in method.params() {
            collect_from_schema(param.schema(), &mut seen, &mut out);
        }
        if let JsApiReturn::Schema(return_schema) = method.returns() {
            collect_from_schema(&return_schema, &mut seen, &mut out);
        }
    }
    out
}

fn collect_from_schema(
    schema: &TypeSchema,
    seen: &mut BTreeSet<&'static str>,
    out: &mut Vec<NamedType>,
) {
    match schema {
        TypeSchema::Option(inner)
        | TypeSchema::List(inner)
        | TypeSchema::Signal(inner)
        | TypeSchema::Accessor(inner) => collect_from_schema(inner, seen, out),
        TypeSchema::Array { item, .. } => collect_from_schema(item, seen, out),
        TypeSchema::Map { key, value } => {
            collect_from_schema(key, seen, out);
            collect_from_schema(value, seen, out);
        }
        TypeSchema::Callback(args) | TypeSchema::Union(args) => {
            for item in *args {
                collect_from_schema(item, seen, out);
            }
        }
        TypeSchema::Struct(struct_schema) if seen.insert(struct_schema.name()) => {
            out.push(NamedType::Struct(*struct_schema));
            for field in struct_schema.fields() {
                collect_from_schema(field.ty(), seen, out);
            }
        }
        TypeSchema::Enum(enum_schema) if seen.insert(enum_schema.name()) => {
            out.push(NamedType::Enum(*enum_schema));
            for variant in enum_schema.variants() {
                match variant.payload() {
                    VariantPayload::Tuple(items) => {
                        for item in items {
                            collect_from_schema(item, seen, out);
                        }
                    }
                    VariantPayload::Struct(fields) => {
                        for field in fields {
                            collect_from_schema(field.ty(), seen, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn render_struct_decl(out: &mut String, schema: StructSchema) {
    let _ = writeln!(out, "interface {} {{", schema.name());
    for field in schema.fields() {
        let _ = writeln!(out, "  {}: {};", field.name(), render_type(field.ty()));
    }
    let _ = writeln!(out, "}}");
}

fn render_enum_decl(out: &mut String, schema: EnumSchema) {
    let body = if schema.variants().is_empty() {
        String::from("never")
    } else {
        match schema.representation() {
            EnumRepresentation::StringUnion => schema
                .variants()
                .iter()
                .map(|variant| format!("{:?}", variant.name()))
                .collect::<Vec<_>>()
                .join(" | "),
            EnumRepresentation::Tagged { tag, content } => schema
                .variants()
                .iter()
                .map(|variant| {
                    render_tagged_variant(tag, content, variant.name(), variant.payload())
                })
                .collect::<Vec<_>>()
                .join(" | "),
        }
    };
    let _ = writeln!(out, "type {} = {body};", schema.name());
}

fn render_tagged_variant(tag: &str, content: &str, name: &str, payload: VariantPayload) -> String {
    match payload {
        VariantPayload::Tuple(items) => {
            let rendered = items.iter().map(render_type).collect::<Vec<_>>().join(", ");
            format!("{{ {tag}: {name:?}; {content}: [{rendered}] }}")
        }
        VariantPayload::Struct(fields) => {
            let rendered = render_inline_fields(fields);
            format!("{{ {tag}: {name:?}; {content}: {rendered} }}")
        }
        _ => format!("{{ {tag}: {name:?} }}"),
    }
}

fn render_inline_fields(fields: &[FieldSchema]) -> String {
    if fields.is_empty() {
        return String::from("{}");
    }
    let inner = fields
        .iter()
        .map(|field| format!("{}: {}", field.name(), render_type(field.ty())))
        .collect::<Vec<_>>()
        .join("; ");
    format!("{{ {inner} }}")
}

fn render_members(schema: &JsApiSchema) -> String {
    let state_members: Vec<String> = schema
        .state()
        .iter()
        .map(|field| {
            let (prefix, value_schema) = match field.schema() {
                TypeSchema::Signal(inner) => ("", *inner),
                TypeSchema::Accessor(inner) => ("readonly ", *inner),
                other => ("readonly ", other),
            };
            format!(
                "    {prefix}{:?}: {};",
                field.name(),
                render_type(value_schema)
            )
        })
        .collect();

    let call_members: Vec<String> = schema
        .methods()
        .iter()
        .map(|method| {
            let returns = render_return(method.returns());
            if method.params().is_empty() {
                format!("  invoke(name: {:?}): Promise<{returns}>;", method.name())
            } else {
                let args = method
                    .params()
                    .iter()
                    .map(|param| format!("{}: {}", param.name(), render_type(param.schema())))
                    .collect::<Vec<_>>()
                    .join("; ");
                format!(
                    "  invoke(name: {:?}, args: {{ {args} }}): Promise<{returns}>;",
                    method.name()
                )
            }
        })
        .collect();

    // `state` is always declared, even when empty: the page can still reach it,
    // and an interface that omits it would make `waterui.state` a type error.
    format!(
        "  state: {{\n{}\n  }};\n{}",
        state_members.join("\n"),
        call_members.join("\n")
    )
}

fn render_return(returns: JsApiReturn) -> String {
    match returns {
        JsApiReturn::Schema(schema) => render_type(&schema),
        JsApiReturn::Bytes => String::from("Uint8Array"),
    }
}

/// Whether `schema` renders with a top-level union or function arrow that needs
/// parentheses inside `T[]`.
const fn needs_list_parens(schema: &TypeSchema) -> bool {
    match schema {
        TypeSchema::Union(_)
        | TypeSchema::Option(_)
        | TypeSchema::Callback(_)
        | TypeSchema::ViewBuilder => true,
        TypeSchema::Number(kind) => kind.is_bigint(),
        _ => false,
    }
}

/// Renders a [`TypeSchema`] as the TypeScript type expression used on the
/// webview bridge.
///
/// Unlike the `waterui-ts` props seam — where 64-bit integers always cross as
/// `bigint` — the webview bridge tags integers dynamically only past `2^53`
/// (`crate::big_integers`), so 64-bit and pointer-sized integers project to
/// `number | bigint`.
fn render_type(schema: &TypeSchema) -> String {
    match schema {
        TypeSchema::Unit => String::from("void"),
        TypeSchema::Bool => String::from("boolean"),
        TypeSchema::Number(kind) if kind.is_bigint() => String::from("number | bigint"),
        TypeSchema::Number(_) => String::from("number"),
        TypeSchema::String => String::from("string"),
        TypeSchema::Option(inner) => format!("{} | null", render_type(inner)),
        TypeSchema::List(inner) if needs_list_parens(inner) => {
            format!("({})[]", render_type(inner))
        }
        TypeSchema::List(inner) => format!("{}[]", render_type(inner)),
        TypeSchema::Array { item, len } => {
            let item = render_type(item);
            let items = vec![item; *len].join(", ");
            format!("[{items}]")
        }
        TypeSchema::Map { key, value } => {
            format!("Record<{}, {}>", render_type(key), render_type(value))
        }
        TypeSchema::Signal(inner) => format!("Signal<{}>", render_type(inner)),
        TypeSchema::Accessor(inner) => format!("Accessor<{}>", render_type(inner)),
        TypeSchema::View => String::from("View"),
        TypeSchema::ViewBuilder => String::from("() => JSX.Element"),
        TypeSchema::Callback(args) => {
            let params = args
                .iter()
                .enumerate()
                .map(|(index, arg)| format!("arg{index}: {}", render_type(arg)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({params}) => void")
        }
        TypeSchema::Union(members) => members
            .iter()
            .map(render_type)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeSchema::Struct(schema) => schema.name().to_owned(),
        TypeSchema::Enum(schema) => schema.name().to_owned(),
        _ => String::from("unknown"),
    }
}
