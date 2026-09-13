//! Tool argument types, the [`ToolDispatch`] contract, and the `Tool`
//! implementations that forward each call to a dispatch.
//!
//! The argument types carry the rustdoc a model reads as the tool and field
//! descriptions, and derive [`Serialize`] so a front can forward them to a
//! child process as JSON.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;

use aither_core::llm::tool::{Tool, ToolResult, Tools};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Implements [`Tool`] for a dispatch-backed tool: `call` forwards the parsed
/// arguments to [`ToolDispatch::$method`] and returns its [`ToolResult`].
macro_rules! session_tool {
    (
        $(#[$meta:meta])*
        $tool:ident, $name:literal, $args:ty, $method:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $tool<D>(Arc<D>);

        impl<D: ToolDispatch> $tool<D> {
            /// Binds the tool to `dispatch`.
            pub const fn new(dispatch: Arc<D>) -> Self {
                Self(dispatch)
            }
        }

        impl<D: ToolDispatch> Tool for $tool<D> {
            type Arguments = $args;
            type Res = ToolResult;

            fn name(&self) -> Cow<'static, str> {
                $name.into()
            }

            fn call(
                &self,
                args: Self::Arguments,
            ) -> impl Future<Output = aither_core::Result<Self::Res>> + Send {
                let dispatch = self.0.clone();
                async move { Ok(dispatch.$method(args).await) }
            }
        }
    };
}

/// One implementation per host: the in-process session (`waterui-mcp`) and the
/// `water mcp` front that forwards calls to a child process.
///
/// Every method answers with a [`ToolResult`]; transport-level failures are
/// reported through [`ToolResult::error`] so they reach the model as ordinary
/// tool errors.
pub trait ToolDispatch: Send + Sync + 'static {
    /// Read the accessibility tree.
    fn snapshot(&self, args: SnapshotArgs) -> impl Future<Output = ToolResult> + Send;
    /// Find nodes matching a selector.
    fn find(&self, args: FindArgs) -> impl Future<Output = ToolResult> + Send;
    /// Dispatch a semantic accessibility action.
    fn act(&self, args: ActArgs) -> impl Future<Output = ToolResult> + Send;
    /// Dispatch a pointer event.
    fn pointer(&self, args: PointerArgs) -> impl Future<Output = ToolResult> + Send;
    /// Dispatch a key press.
    fn key(&self, args: KeyArgs) -> impl Future<Output = ToolResult> + Send;
    /// Type text into the focused input.
    fn type_text(&self, args: TypeTextArgs) -> impl Future<Output = ToolResult> + Send;
    /// Wait for expectations.
    fn wait(&self, args: WaitArgs) -> impl Future<Output = ToolResult> + Send;
    /// Capture a PNG screenshot.
    fn screenshot(&self, args: ScreenshotArgs) -> impl Future<Output = ToolResult> + Send;
    /// Relaunch the app and return the fresh tree.
    fn restart(&self, args: RestartArgs) -> impl Future<Output = ToolResult> + Send;
}

/// Read the accessibility tree: every node's id, role, label, value, state
/// flags, supported actions, and bounds.
///
/// Mutating tools already return the settled tree, so call this to re-read
/// state without acting.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SnapshotArgs {
    /// `text` (default) renders indented node lines; `json` renders the full
    /// node tree as JSON.
    pub format: SnapshotFormat,
}

/// Output format for the `snapshot` tool.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotFormat {
    /// Indented text lines, one per node.
    #[default]
    Text,
    /// Nested JSON mirroring the node tree.
    Json,
}

/// Criteria matching nodes by their accessibility properties.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SelectorArgs {
    /// Role name as `snapshot` prints it: `button`, `text_input`,
    /// `scroll_view`, …
    pub role: Option<String>,
    /// Exact accessibility label.
    pub label: Option<String>,
    /// Substring the label must contain.
    pub label_contains: Option<String>,
    /// Accessibility identifier (`a11y_id`).
    pub identifier: Option<String>,
    /// Exact current value.
    pub value: Option<String>,
}

/// Find nodes matching the criteria and return their node lines. Fails when
/// nothing matches.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FindArgs {
    /// The match criteria, inlined into the tool arguments.
    #[serde(flatten)]
    pub criteria: SelectorArgs,
}

/// Perform a semantic accessibility action on a node, then return the settled
/// tree. Prefer this over `pointer` — it works regardless of layout.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ActArgs {
    /// Target node id from a `snapshot` or `find` line (`#42` → `42`).
    pub node: u64,
    /// The action to perform.
    pub action: ActAction,
    /// The value for `set_value` and `replace_text`; required for them,
    /// ignored by other actions.
    #[serde(default)]
    pub value: Option<String>,
}

/// The accessibility actions `act` can dispatch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActAction {
    /// Activate the node — a click or tap.
    Click,
    /// Move accessibility focus to the node.
    Focus,
    /// Replace the node's value; numeric controls take a numeric value.
    SetValue,
    /// Step a numeric value up.
    Increment,
    /// Step a numeric value down.
    Decrement,
    /// Replace the node's selected text with `value`.
    ReplaceText,
    /// Expand a collapsed node.
    Expand,
    /// Collapse an expanded node.
    Collapse,
    /// Scroll the node forward (down/right).
    ScrollForward,
    /// Scroll the node backward (up/left).
    ScrollBackward,
}

impl ActAction {
    /// The wire name, matching the `serde` spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Focus => "focus",
            Self::SetValue => "set_value",
            Self::Increment => "increment",
            Self::Decrement => "decrement",
            Self::ReplaceText => "replace_text",
            Self::Expand => "expand",
            Self::Collapse => "collapse",
            Self::ScrollForward => "scroll_forward",
            Self::ScrollBackward => "scroll_backward",
        }
    }
}

/// Dispatch a pointer event at viewport coordinates, then return the settled
/// tree. Coordinates are logical pixels, matching the `bounds=` values in
/// `snapshot` output.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PointerArgs {
    /// Which pointer event to dispatch.
    pub kind: PointerKind,
    /// X coordinate in logical pixels.
    pub x: f32,
    /// Y coordinate in logical pixels.
    pub y: f32,
    /// Drag end X — required when `kind` is `drag`.
    #[serde(default)]
    pub to_x: Option<f32>,
    /// Drag end Y — required when `kind` is `drag`.
    #[serde(default)]
    pub to_y: Option<f32>,
    /// Horizontal scroll delta — used when `kind` is `scroll`.
    #[serde(default)]
    pub dx: Option<f32>,
    /// Vertical scroll delta — used when `kind` is `scroll`.
    #[serde(default)]
    pub dy: Option<f32>,
    /// Number of intermediate positions for `drag`.
    #[serde(default)]
    pub steps: Option<u16>,
}

/// The pointer events `pointer` can dispatch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PointerKind {
    /// Primary down+up at the point.
    Tap,
    /// Primary button down.
    Down,
    /// Primary button up.
    Up,
    /// Move the pointer without changing hover-driven state semantics.
    Move,
    /// Hover at the point.
    Hover,
    /// Secondary (right) click.
    SecondaryClick,
    /// Drag from `x`,`y` to `to_x`,`to_y`.
    Drag,
    /// Scroll wheel at the point, by `dx`/`dy` pixels.
    Scroll,
}

/// Press a key, then return the settled tree.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct KeyArgs {
    /// A single character (`"a"`, `" "`), or a W3C named key: `Enter`, `Tab`,
    /// `Escape`, `Backspace`, `Delete`, `ArrowLeft`, `ArrowRight`, `ArrowUp`,
    /// `ArrowDown`, `Home`, `End`, `PageUp`, `PageDown`, `F1`–`F12`.
    pub key: String,
    /// Modifiers held during the press: `shift`, `ctrl`, `alt`, `meta`.
    #[serde(default)]
    pub modifiers: Vec<String>,
}

/// Type text into the focused text input, then return the settled tree.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TypeTextArgs {
    /// The text to insert.
    pub text: String,
}

/// Wait until every given expectation holds, then return `fulfilled` — or
/// `timed out` — followed by the current tree. Prefer this over polling
/// `snapshot`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WaitArgs {
    /// Fulfilled once a matching node exists.
    pub exists: Option<SelectorArgs>,
    /// Fulfilled once no matching node exists.
    pub not_exists: Option<SelectorArgs>,
    /// Fulfilled once a matching node's value equals `value`.
    pub value_eq: Option<ValueEqArgs>,
    /// Timeout in milliseconds; defaults to 5000.
    pub timeout_ms: Option<u64>,
}

/// A selector plus the value a matching node must reach.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ValueEqArgs {
    /// The match criteria, inlined into the tool arguments.
    #[serde(flatten)]
    pub selector: SelectorArgs,
    /// The expected value.
    pub value: String,
}

/// Capture the current frame as a PNG image.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {}

/// Relaunch the app from scratch and return the fresh tree; state resets.
///
/// Under `water mcp` the app is rebuilt from the current sources first, so
/// edit → `restart` → `snapshot` is the development loop. Node ids from
/// before the restart no longer refer to live nodes.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RestartArgs {}

session_tool!(
    /// The `snapshot` tool.
    Snapshot, "snapshot", SnapshotArgs, snapshot
);
session_tool!(
    /// The `find` tool.
    Find, "find", FindArgs, find
);
session_tool!(
    /// The `act` tool.
    Act, "act", ActArgs, act
);
session_tool!(
    /// The `pointer` tool.
    Pointer, "pointer", PointerArgs, pointer
);
session_tool!(
    /// The `key` tool.
    Key, "key", KeyArgs, key
);
session_tool!(
    /// The `type_text` tool.
    TypeText, "type_text", TypeTextArgs, type_text
);
session_tool!(
    /// The `wait` tool.
    Wait, "wait", WaitArgs, wait
);
session_tool!(
    /// The `screenshot` tool.
    Screenshot, "screenshot", ScreenshotArgs, screenshot
);
session_tool!(
    /// The `restart` tool.
    Restart, "restart", RestartArgs, restart
);

/// The registered tool names, in registration order.
pub const SESSION_TOOL_NAMES: [&str; 9] = [
    "snapshot",
    "find",
    "act",
    "pointer",
    "key",
    "type_text",
    "wait",
    "screenshot",
    "restart",
];

/// Registers the nine session tools against `dispatch`.
///
/// # Panics
///
/// Registration is static — fixed names and documented argument types — so a
/// failure here is a programming error, not a runtime condition.
pub fn register_session_tools(tools: &mut Tools, dispatch: Arc<impl ToolDispatch>) {
    tools
        .register(Snapshot::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Find::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Act::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Pointer::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Key::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(TypeText::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Wait::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Screenshot::new(dispatch.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Restart::new(dispatch))
        .expect("static tool registration cannot fail");
}
