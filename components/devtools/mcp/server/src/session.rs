//! The session thread: owns the `!Send` [`OffscreenApp`] and executes every
//! tool request as a [`Command`] delivered over a channel.

use std::future::Future;
use std::time::Duration;

use accesskit::{Action, ActionData, NodeId as AccessibilityNodeId};
use aither_core::llm::tool::ToolResult;
use async_channel::Sender;
use waterui_testing::{DragOptions, NodeId, OffscreenApp, Role, Selector, WaitOptions, WaitResult};

use waterui_mcp_protocol::{
    ActAction, ActArgs, FindArgs, KeyArgs, PointerArgs, PointerKind, SelectorArgs, SnapshotArgs,
    SnapshotFormat, ScreenshotArgs, RestartArgs, ToolDispatch, TypeTextArgs, WaitArgs,
};

use crate::tree;

/// Where a finished command's [`ToolResult`] travels back to its tool handler.
type Reply = Sender<ToolResult>;

/// One unit of work for the session thread.
#[derive(Debug)]
pub enum Command {
    /// Render the accessibility tree.
    Snapshot {
        /// Text or JSON form.
        format: SnapshotFormat,
        /// Reply channel.
        reply: Reply,
    },
    /// Find nodes matching a selector.
    Find {
        /// Search criteria.
        args: FindArgs,
        /// Reply channel.
        reply: Reply,
    },
    /// Dispatch a semantic accessibility action.
    Act {
        /// Node, action, optional value.
        args: ActArgs,
        /// Reply channel.
        reply: Reply,
    },
    /// Dispatch a pointer event.
    Pointer {
        /// Kind, coordinates, deltas.
        args: PointerArgs,
        /// Reply channel.
        reply: Reply,
    },
    /// Dispatch a key press.
    Key {
        /// Key name or character plus modifiers.
        args: KeyArgs,
        /// Reply channel.
        reply: Reply,
    },
    /// Type text into the focused input.
    TypeText {
        /// Text to type.
        text: String,
        /// Reply channel.
        reply: Reply,
    },
    /// Wait for expectations.
    Wait {
        /// Expectations and timeout — boxed, it dwarfs the other payloads.
        args: Box<WaitArgs>,
        /// Reply channel.
        reply: Reply,
    },
    /// Capture a PNG screenshot.
    Screenshot {
        /// Reply channel.
        reply: Reply,
    },
    /// Remount the app.
    Restart {
        /// Reply channel.
        reply: Reply,
    },
}

/// Cloneable handle a tool uses to reach the session thread.
///
/// Dropping every handle — which happens when the MCP server exits and its
/// tool registry drops — closes the channel and ends the session loop.
/// Commands are boxed: the variants' argument records differ widely in size.
#[derive(Clone, Debug)]
pub struct SessionHandle(Sender<Box<Command>>);

impl SessionHandle {
    pub const fn new(sender: Sender<Box<Command>>) -> Self {
        Self(sender)
    }

    /// Sends a command and awaits its result.
    ///
    /// # Errors
    ///
    /// Fails only when the session has ended: each reply channel is
    /// `bounded(1)`, so a live session can always answer.
    pub async fn request(
        &self,
        command: impl FnOnce(Reply) -> Command,
    ) -> aither_core::Result<ToolResult> {
        let (reply, rx) = async_channel::bounded(1);
        self.0
            .send(Box::new(command(reply)))
            .await
            .map_err(|_| aither_core::Error::msg("waterui-mcp session has ended"))?;
        rx.recv()
            .await
            .map_err(|_| aither_core::Error::msg("waterui-mcp session dropped a command"))
    }
}

impl ToolDispatch for SessionHandle {
    fn snapshot(&self, args: SnapshotArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Snapshot {
                    format: args.format,
                    reply,
                })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn find(&self, args: FindArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Find { args, reply })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn act(&self, args: ActArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Act { args, reply })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn pointer(&self, args: PointerArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Pointer { args, reply })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn key(&self, args: KeyArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Key { args, reply })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn type_text(&self, args: TypeTextArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::TypeText {
                    text: args.text,
                    reply,
                })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn wait(&self, args: WaitArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Wait {
                    args: Box::new(args),
                    reply,
                })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn screenshot(&self, _args: ScreenshotArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Screenshot { reply })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }

    fn restart(&self, _args: RestartArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Restart { reply })
                .await
                .unwrap_or_else(|error| ToolResult::error(error.to_string()))
        }
    }
}

/// Roles [`Selector::role`] can take, mapped from the `snake_case` names the
/// tree prints. AccessKit offers no name parsing, and its `Role` enum has no
/// stable variant list, so the searchable names are exactly the roles
/// waterui-testing exposes.
const NAMED_ROLES: &[Role] = &[
    Role::BUTTON,
    Role::LABEL,
    Role::TEXT_INPUT,
    Role::PASSWORD_INPUT,
    Role::CHECKBOX,
    Role::SWITCH,
    Role::SLIDER,
    Role::IMAGE,
    Role::SCROLL_VIEW,
    Role::LIST,
    Role::LIST_ITEM,
    Role::TAB,
    Role::TAB_LIST,
    Role::COMBOBOX,
    Role::OPTION,
    Role::MULTILINE_TEXT_INPUT,
    Role::LINK,
    Role::HEADER,
    Role::FOOTER,
    Role::PROGRESS_INDICATOR,
    Role::SPIN_BUTTON,
    Role::RADIO_BUTTON,
    Role::MENU,
    Role::MENU_BAR,
    Role::MENU_ITEM,
    Role::MENU_ITEM_CHECKBOX,
    Role::MENU_ITEM_RADIO,
    Role::TAB_PANEL,
    Role::TABLE,
    Role::CELL,
    Role::COLUMN_HEADER,
    Role::GROUP,
    Role::WINDOW,
    Role::MAIN,
    Role::NAVIGATION,
    Role::SEARCH,
    Role::ARTICLE,
    Role::SECTION,
];

/// Resolves a `snake_case` role name to a [`Role`].
///
/// # Errors
///
/// Returns a message naming the unknown role and listing the searchable ones.
fn resolve_role(name: &str) -> Result<Role, String> {
    NAMED_ROLES
        .iter()
        .copied()
        .find(|role| tree::role_name(*role) == name)
        .ok_or_else(|| {
            let known = NAMED_ROLES
                .iter()
                .map(|role| tree::role_name(*role))
                .collect::<Vec<_>>()
                .join(", ");
            format!("unknown role `{name}`; known roles: {known}")
        })
}

/// Builds the [`Selector`] `wait`'s expectations take.
///
/// # Errors
///
/// Returns a message when `role` is not a searchable name.
fn selector_from(args: &SelectorArgs) -> Result<Selector, String> {
    let mut selector = Selector::default();
    if let Some(role) = &args.role {
        selector = selector.role(resolve_role(role)?);
    }
    if let Some(label) = &args.label {
        selector = selector.label(label.clone());
    }
    if let Some(label_contains) = &args.label_contains {
        selector = selector.label_contains(label_contains.clone());
    }
    if let Some(identifier) = &args.identifier {
        selector = selector.identifier(identifier.clone());
    }
    if let Some(value) = &args.value {
        selector = selector.value(value.clone());
    }
    Ok(selector)
}

/// Applies the same criteria to a [`Query`]; `find` searches through the query
/// API, which mirrors `Selector` but does not consume one.
///
/// # Errors
///
/// Returns a message when `role` is not a searchable name.
fn query_from<'a>(
    app: &'a mut waterui_testing::SemanticApp,
    args: &SelectorArgs,
) -> Result<waterui_testing::Query<'a>, String> {
    let mut query = app.query();
    if let Some(role) = &args.role {
        query = query.role(resolve_role(role)?);
    }
    if let Some(label) = &args.label {
        query = query.label(label.clone());
    }
    if let Some(label_contains) = &args.label_contains {
        query = query.label_contains(label_contains.clone());
    }
    if let Some(identifier) = &args.identifier {
        query = query.identifier(identifier.clone());
    }
    if let Some(value) = &args.value {
        query = query.value(value.clone());
    }
    Ok(query)
}

/// Numeric-valued roles whose `set_value` data travels as
/// [`ActionData::NumericValue`].
const NUMERIC_ROLES: &[Role] = &[Role::SLIDER, Role::SPIN_BUTTON, Role::PROGRESS_INDICATOR];

/// Owns the mounted app on the session thread and executes [`Command`]s
/// against it. Every mutating command settles and answers with the fresh tree,
/// so agents never need a follow-up `snapshot`.
pub struct Session<'a> {
    app: OffscreenApp,
    mount: Box<dyn FnMut() -> OffscreenApp + 'a>,
}

impl<'a> Session<'a> {
    /// Mounts the app once on this thread.
    pub fn new(mount: impl FnMut() -> OffscreenApp + 'a) -> Self {
        let mut mount = mount;
        Self {
            app: mount(),
            mount: Box::new(mount),
        }
    }

    /// Executes one command and replies on its channel. A dropped receiver —
    /// the tool future was cancelled — simply discards the result.
    pub fn execute(&mut self, command: Box<Command>) {
        tracing::debug!(?command, "waterui-mcp executing command");
        match *command {
            Command::Snapshot { format, reply } => {
                let _ = reply.try_send(self.snapshot(format));
            }
            Command::Find { args, reply } => {
                let _ = reply.try_send(self.find(&args));
            }
            Command::Act { args, reply } => {
                let _ = reply.try_send(self.act(&args));
            }
            Command::Pointer { args, reply } => {
                let _ = reply.try_send(self.pointer(&args));
            }
            Command::Key { args, reply } => {
                let _ = reply.try_send(self.key(&args));
            }
            Command::TypeText { text, reply } => {
                self.app.text_input(text);
                let _ = reply.try_send(self.tree_text());
            }
            Command::Wait { args, reply } => {
                let _ = reply.try_send(self.wait(&args));
            }
            Command::Screenshot { reply } => {
                let _ = reply.try_send(self.screenshot());
            }
            Command::Restart { reply } => {
                self.app = (self.mount)();
                let _ = reply.try_send(self.tree_text());
            }
        }
    }

    fn tree_text(&self) -> ToolResult {
        ToolResult::text(tree::render_text(&self.app))
    }

    fn snapshot(&self, format: SnapshotFormat) -> ToolResult {
        match format {
            SnapshotFormat::Text => self.tree_text(),
            SnapshotFormat::Json => match tree::render_json(&self.app) {
                Ok(json) => ToolResult::text(json),
                Err(error) => ToolResult::error(format!("failed to serialize tree: {error}")),
            },
        }
    }

    fn find(&mut self, args: &FindArgs) -> ToolResult {
        let focus = self.app.tree().focus();
        let query = match query_from(&mut self.app, &args.criteria) {
            Ok(query) => query,
            Err(error) => return ToolResult::error(error),
        };
        let lines = query
            .all()
            .iter()
            .map(|element| tree::node_line(element.node(), element.id() == focus))
            .collect::<Vec<_>>();
        if lines.is_empty() {
            ToolResult::error("no nodes matched")
        } else {
            ToolResult::text(lines.join("\n"))
        }
    }

    fn act(&mut self, args: &ActArgs) -> ToolResult {
        let node_id = NodeId::from(AccessibilityNodeId(args.node));
        let revision = self.app.tree().revision();
        let Some(node) = self.app.tree().node(node_id).cloned() else {
            return ToolResult::error(format!(
                "unknown node #{} at revision {revision}",
                args.node
            ));
        };
        let (action, data) = match act_request(args, &node) {
            Ok(request) => request,
            Err(error) => return ToolResult::error(error),
        };
        if !node.actions().contains(&action) {
            let supported = node
                .actions()
                .iter()
                .map(|action| tree::snake_case(&format!("{action:?}")))
                .collect::<Vec<_>>()
                .join(", ");
            return ToolResult::error(format!(
                "node {} does not support `{}`; supported actions: {supported}",
                tree::node_line(&node, self.app.tree().focus() == node_id),
                args.action.as_str(),
            ));
        }
        if !self.app.perform_action(node_id, action, data) {
            return ToolResult::error(format!(
                "the runtime did not handle `{}` on node #{}",
                args.action.as_str(),
                args.node
            ));
        }
        self.tree_text()
    }

    fn pointer(&mut self, args: &PointerArgs) -> ToolResult {
        match args.kind {
            PointerKind::Tap => self.app.tap_at(args.x, args.y),
            PointerKind::Down => self.app.pointer_down_at(args.x, args.y),
            PointerKind::Up => self.app.pointer_up_at(args.x, args.y),
            PointerKind::Move => {
                self.app.queue_pointer_move(args.x, args.y);
                self.app.settle();
            }
            PointerKind::Hover => self.app.hover_at(args.x, args.y),
            PointerKind::SecondaryClick => self.app.secondary_click_at(args.x, args.y),
            PointerKind::Drag => {
                let (Some(to_x), Some(to_y)) = (args.to_x, args.to_y) else {
                    return ToolResult::error("`drag` requires `to_x` and `to_y`");
                };
                let steps = args
                    .steps
                    .map_or_else(DragOptions::default, |steps| DragOptions {
                        steps,
                        ..DragOptions::default()
                    });
                self.app
                    .drag_from_to_with(args.x, args.y, to_x, to_y, steps);
            }
            PointerKind::Scroll => {
                self.app.scroll_at(
                    args.x,
                    args.y,
                    args.dx.unwrap_or(0.0),
                    args.dy.unwrap_or(0.0),
                    false,
                );
            }
        }
        self.tree_text()
    }

    fn key(&mut self, args: &KeyArgs) -> ToolResult {
        let mut modifiers = waterui_testing::Modifiers::default();
        for name in &args.modifiers {
            match name.as_str() {
                "shift" => modifiers.shift = true,
                "ctrl" => modifiers.control = true,
                "alt" => modifiers.alt = true,
                "meta" => modifiers.super_key = true,
                other => {
                    return ToolResult::error(format!(
                        "unknown modifier `{other}`; expected shift, ctrl, alt, or meta"
                    ));
                }
            }
        }
        let mut chars = args.key.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            self.app.press_character_key_with(ch.to_string(), modifiers);
        } else {
            self.app.press_named_key_with(args.key.clone(), modifiers);
        }
        self.tree_text()
    }

    fn wait(&mut self, args: &WaitArgs) -> ToolResult {
        let mut expectations = Vec::new();
        if let Some(criteria) = &args.exists {
            match selector_from(criteria) {
                Ok(selector) => expectations.push(self.app.expect_exists(selector)),
                Err(error) => return ToolResult::error(error),
            }
        }
        if let Some(criteria) = &args.not_exists {
            match selector_from(criteria) {
                Ok(selector) => expectations.push(self.app.expect_not_exists(selector)),
                Err(error) => return ToolResult::error(error),
            }
        }
        if let Some(value_eq) = &args.value_eq {
            match selector_from(&value_eq.selector) {
                Ok(selector) => {
                    expectations.push(self.app.expect_value_eq(selector, value_eq.value.clone()));
                }
                Err(error) => return ToolResult::error(error),
            }
        }
        if expectations.is_empty() {
            return ToolResult::error(
                "`wait` requires at least one of `exists`, `not_exists`, or `value_eq`",
            );
        }
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(5000));
        let status = match self.app.wait_for(&expectations, WaitOptions::new(timeout)) {
            WaitResult::Completed => "fulfilled",
            WaitResult::TimedOut => "timed out",
            WaitResult::IncorrectOrder => "incorrect_order",
            WaitResult::InvertedFulfillment => "inverted_fulfillment",
            WaitResult::Interrupted => "interrupted",
        };
        ToolResult::text(format!("{status}\n\n{}", tree::render_text(&self.app)))
    }

    fn screenshot(&mut self) -> ToolResult {
        let mut snapshot = self.app.snapshot();
        crate::png::flatten_alpha_over_white(&mut snapshot.rgba8);
        match crate::png::encode(&snapshot) {
            Ok(bytes) => ToolResult::image(bytes, "image/png"),
            Err(error) => ToolResult::error(format!("failed to encode screenshot: {error}")),
        }
    }
}

/// Maps an `act` request onto an AccessKit action and its optional data.
///
/// # Errors
///
/// Returns a message for an unknown action name, a `set_value` /
/// `replace_text` call missing its `value`, or a `set_value` on a
/// numeric-role node whose `value` does not parse as `f64`.
fn act_request(
    args: &ActArgs,
    node: &waterui_testing::NodeSnapshot,
) -> Result<(Action, Option<ActionData>), String> {
    let value = || {
        args.value
            .clone()
            .ok_or_else(|| format!("action `{}` requires `value`", args.action.as_str()))
    };
    match args.action {
        ActAction::Click => Ok((Action::Click, None)),
        ActAction::Focus => Ok((Action::Focus, None)),
        ActAction::Increment => Ok((Action::Increment, None)),
        ActAction::Decrement => Ok((Action::Decrement, None)),
        ActAction::Expand => Ok((Action::Expand, None)),
        ActAction::Collapse => Ok((Action::Collapse, None)),
        ActAction::ScrollForward => Ok((Action::ScrollDown, None)),
        ActAction::ScrollBackward => Ok((Action::ScrollUp, None)),
        ActAction::ReplaceText => Ok((
            Action::ReplaceSelectedText,
            Some(ActionData::Value(value()?.into())),
        )),
        ActAction::SetValue => {
            let value = value()?;
            let data = if NUMERIC_ROLES.iter().any(|role| *role == node.role()) {
                ActionData::NumericValue(value.parse::<f64>().map_err(|_| {
                    format!(
                        "`set_value` on a {} needs a numeric value, got `{value}`",
                        tree::role_name(node.role())
                    )
                })?)
            } else {
                ActionData::Value(value.into())
            };
            Ok((Action::SetValue, Some(data)))
        }
    }
}
