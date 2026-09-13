//! Tool argument types and the `Tool` implementations that forward each call
//! to the session thread as a [`Command`].

use std::borrow::Cow;
use std::future::Future;

use aither_core::llm::tool::{Tool, ToolResult, Tools};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::session::{Command, SessionHandle};

/// Implements [`Tool`] for a session-backed tool: `call` ships the built
/// [`Command`] to the session thread and awaits the reply.
macro_rules! session_tool {
    (
        $(#[$meta:meta])*
        $tool:ident, $name:literal, $args:ty, |$a:ident, $reply:ident| $command:expr
    ) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $tool(SessionHandle);

        impl $tool {
            pub const fn new(handle: SessionHandle) -> Self {
                Self(handle)
            }
        }

        impl Tool for $tool {
            type Arguments = $args;
            type Res = ToolResult;

            fn name(&self) -> Cow<'static, str> {
                $name.into()
            }

            fn call(
                &self,
                $a: Self::Arguments,
            ) -> impl Future<Output = aither_core::Result<Self::Res>> + Send {
                let session = self.0.clone();
                async move { session.request(|$reply| $command).await }
            }
        }
    };
}

/// Read the accessibility tree: every node's id, role, label, value, state
/// flags, supported actions, and bounds. Mutating tools already return the
/// settled tree, so call this to re-read state without acting.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SnapshotArgs {
    /// `text` (default) renders indented node lines; `json` renders the full
    /// node tree as JSON.
    pub format: SnapshotFormat,
}

/// Output format for the `snapshot` tool.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotFormat {
    /// Indented text lines, one per node.
    #[default]
    Text,
    /// Nested JSON mirroring the node tree.
    Json,
}

/// Criteria matching nodes by their accessibility properties.
#[derive(Debug, Default, Deserialize, JsonSchema)]
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
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct FindArgs {
    /// The match criteria, inlined into the tool arguments.
    #[serde(flatten)]
    pub criteria: SelectorArgs,
}

/// Perform a semantic accessibility action on a node, then return the settled
/// tree. Prefer this over `pointer` — it works regardless of layout.
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TypeTextArgs {
    /// The text to insert.
    pub text: String,
}

/// Wait until every given expectation holds, then return `fulfilled` — or
/// `timed out` — followed by the current tree. Prefer this over polling
/// `snapshot`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
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
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValueEqArgs {
    /// The match criteria, inlined into the tool arguments.
    #[serde(flatten)]
    pub selector: SelectorArgs,
    /// The expected value.
    pub value: String,
}

/// Capture the current frame as a PNG image.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotArgs {}

/// Remount the app from scratch and return the fresh tree. Node ids from
/// before the restart no longer refer to live nodes.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestartArgs {}

session_tool!(
    /// The `snapshot` tool.
    Snapshot, "snapshot", SnapshotArgs, |args, reply| Command::Snapshot {
        format: args.format,
        reply
    }
);
session_tool!(
    /// The `find` tool.
    Find, "find", FindArgs, |args, reply| Command::Find { args, reply }
);
session_tool!(
    /// The `act` tool.
    Act, "act", ActArgs, |args, reply| Command::Act { args, reply }
);
session_tool!(
    /// The `pointer` tool.
    Pointer, "pointer", PointerArgs, |args, reply| Command::Pointer { args, reply }
);
session_tool!(
    /// The `key` tool.
    Key, "key", KeyArgs, |args, reply| Command::Key { args, reply }
);
session_tool!(
    /// The `type_text` tool.
    TypeText, "type_text", TypeTextArgs, |args, reply| Command::TypeText {
        text: args.text,
        reply
    }
);
session_tool!(
    /// The `wait` tool.
    Wait, "wait", WaitArgs, |args, reply| Command::Wait {
        args: Box::new(args),
        reply
    }
);
session_tool!(
    /// The `screenshot` tool.
    Screenshot, "screenshot", ScreenshotArgs, |_args, reply| Command::Screenshot {
        reply
    }
);
session_tool!(
    /// The `restart` tool.
    Restart, "restart", RestartArgs, |_args, reply| Command::Restart { reply }
);

/// Registers every tool against `handle`.
///
/// # Panics
///
/// Registration is static — fixed names and documented argument types — so a
/// failure here is a programming error, not a runtime condition.
pub fn register_all(tools: &mut Tools, handle: &SessionHandle) {
    tools
        .register(Snapshot::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Find::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Act::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Pointer::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Key::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(TypeText::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Wait::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Screenshot::new(handle.clone()))
        .expect("static tool registration cannot fail");
    tools
        .register(Restart::new(handle.clone()))
        .expect("static tool registration cannot fail");
}
