//! `waterui-mcp-protocol`: the tool contract a headless `WaterUI` session
//! speaks over MCP.
//!
//! Two hosts implement the same nine tools — `snapshot`, `find`, `act`,
//! `pointer`, `key`, `type_text`, `wait`, `screenshot`, and `restart`:
//!
//! - `waterui-mcp` serves them in-process against a mounted
//!   `waterui_testing::OffscreenApp`.
//! - `water mcp` (in `waterui-cli`) answers `initialize` and `tools/list`
//!   immediately, builds the app's managed Hydrolysis backend in MCP mode, and
//!   forwards `tools/call` to that child once it is up.
//!
//! Both register their tools through [`register_session_tools`] against a
//! [`ToolDispatch`], so the tool names, argument schemas, and descriptions
//! cannot drift between them.

mod tools;

pub use tools::{
    Act, ActAction, ActArgs, Find, FindArgs, Key, KeyArgs, Pointer, PointerArgs, PointerKind,
    Restart, RestartArgs, SESSION_TOOL_NAMES, Screenshot, ScreenshotArgs, SelectorArgs, Snapshot,
    SnapshotArgs, SnapshotFormat, ToolDispatch, TypeText, TypeTextArgs, ValueEqArgs, Wait,
    WaitArgs, register_session_tools,
};

/// Server instructions handed to the client during `initialize`.
pub const INSTRUCTIONS: &str = include_str!("instructions.md");
