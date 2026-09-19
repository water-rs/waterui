//! The session thread: owns the `!Send` [`OffscreenApp`] and executes every
//! tool request as a [`Command`] delivered over a channel.

use std::future::Future;
use std::time::Duration;

use accesskit::{Action, ActionData, NodeId as AccessibilityNodeId};
use aither_core::llm::tool::ToolResult;
use async_channel::Sender;
use waterui_testing::{
    DragOptions, KeyCode, NodeId, OffscreenApp, Role, Selector, VIRTUAL_FRAME, WaitOptions,
    WaitResult,
};

use waterui_mcp_protocol::{
    ActAction, ActArgs, AdvanceArgs, FindArgs, KeyArgs, PointerArgs, PointerKind, RestartArgs,
    ScreenshotArgs, ScrollUnit, SelectorArgs, SnapshotArgs, SnapshotFormat, ToolDispatch,
    TypeTextArgs, WaitArgs,
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
        /// Settle after dispatching.
        settle: Option<bool>,
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
    /// Advance the virtual animation clock.
    Advance {
        /// Duration and capture flag.
        args: AdvanceArgs,
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
                    settle: args.settle,
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

    fn advance(&self, args: AdvanceArgs) -> impl Future<Output = ToolResult> + Send {
        let handle = self.clone();
        async move {
            handle
                .request(|reply| Command::Advance { args, reply })
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

/// Builds the [`Selector`] `find` and `wait`'s expectations take.
///
/// Scope anchors (`within`/`children_of`) resolve against the current tree,
/// which is why the app is borrowed mutably.
///
/// # Errors
///
/// Returns a message when `role` is not a searchable name, when both scope
/// anchors are set, or when a scope anchor does not resolve.
fn selector_from(
    app: &mut waterui_testing::SemanticApp,
    args: &SelectorArgs,
) -> Result<Selector, String> {
    if args.within.is_some() && args.children_of.is_some() {
        return Err("`within` and `children_of` are mutually exclusive".to_owned());
    }
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
    if let Some(value_contains) = &args.value_contains {
        selector = selector.value_contains(value_contains.clone());
    }
    if let Some(enabled) = args.enabled {
        selector = selector.enabled(enabled);
    }
    if let Some(selected) = args.selected {
        selector = selector.selected(selected);
    }
    if let Some(checked) = args.checked {
        selector = selector.checked(checked);
    }
    if args.mixed.unwrap_or(false) {
        selector = selector.mixed();
    }
    if let Some(expanded) = args.expanded {
        selector = selector.expanded(expanded);
    }
    if let Some(busy) = args.busy {
        selector = selector.busy(busy);
    }
    if let Some(hidden) = args.hidden {
        selector = selector.hidden(hidden);
    }
    let mut scope = |id: u64| -> Result<waterui_testing::ElementRef, String> {
        app.element(NodeId::from(AccessibilityNodeId(id)))
            .ok_or_else(|| format!("scope anchor node #{id} is not in the current tree"))
    };
    if let Some(id) = args.within {
        selector = selector.within(scope(id)?);
    }
    if let Some(id) = args.children_of {
        selector = selector.children_of(scope(id)?);
    }
    Ok(selector)
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
            Command::TypeText {
                text,
                settle,
                reply,
            } => {
                self.app.queue_text_input(text);
                let _ = reply.try_send(self.finish_input(settle));
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
            Command::Advance { args, reply } => {
                let _ = reply.try_send(self.advance(&args));
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
        let selector = match selector_from(&mut self.app, &args.criteria) {
            Ok(selector) => selector,
            Err(error) => return ToolResult::error(error),
        };
        let elements = self.app.resolve_elements(&selector);
        let focus = self.app.tree().focus();
        let lines = elements
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
        if matches!(args.action, ActAction::ClearFocus) {
            self.app.queue_clear_ui_focus();
            return self.finish_input(args.settle);
        }
        let Some(node) = args.node else {
            return ToolResult::error(format!("act `{}` requires `node`", args.action.as_str()));
        };
        let node_id = NodeId::from(AccessibilityNodeId(node));
        let revision = self.app.tree().revision();
        let Some(node_snapshot) = self.app.tree().node(node_id).cloned() else {
            return ToolResult::error(format!("unknown node #{node} at revision {revision}"));
        };
        let (action, data) = match act_request(args, &node_snapshot) {
            Ok(request) => request,
            Err(error) => return ToolResult::error(error),
        };
        if !node_snapshot.actions().contains(&action) {
            let supported = node_snapshot
                .actions()
                .iter()
                .map(|action| tree::snake_case(&format!("{action:?}")))
                .collect::<Vec<_>>()
                .join(", ");
            return ToolResult::error(format!(
                "node {} does not support `{}`; supported actions: {supported}",
                tree::node_line(&node_snapshot, self.app.tree().focus() == node_id),
                args.action.as_str(),
            ));
        }
        let handled = self.app.queue_action(node_id, action, data);
        if !handled {
            return ToolResult::error(format!(
                "the runtime did not handle `{}` on node #{node}",
                args.action.as_str(),
            ));
        }
        self.finish_input(args.settle)
    }

    /// Maps `x`/`y` to viewport coordinates: absolute logical pixels, or
    /// fractions of the anchor node's bounds when `node` is set.
    fn anchor_point(
        &mut self,
        node: Option<u64>,
        x: f32,
        y: f32,
    ) -> Result<(f32, f32), ToolResult> {
        let Some(id) = node else {
            return Ok((x, y));
        };
        let Some(element) = self.app.element(NodeId::from(AccessibilityNodeId(id))) else {
            return Err(ToolResult::error(format!(
                "anchor node #{id} is not in the current tree"
            )));
        };
        let Some(bounds) = element.node().bounds() else {
            return Err(ToolResult::error(format!(
                "anchor node #{id} reports no bounds"
            )));
        };
        Ok((
            bounds.width().mul_add(x, bounds.x()),
            bounds.height().mul_add(y, bounds.y()),
        ))
    }

    fn pointer(&mut self, args: &PointerArgs) -> ToolResult {
        let (x, y) = match self.anchor_point(args.node, args.x, args.y) {
            Ok(point) => point,
            Err(result) => return result,
        };
        match args.kind {
            PointerKind::Tap => {
                self.app.queue_pointer_down(x, y);
                self.app.queue_pointer_up(x, y);
            }
            PointerKind::Down => self.app.queue_pointer_down(x, y),
            PointerKind::Up => self.app.queue_pointer_up(x, y),
            PointerKind::Move => self.app.queue_pointer_move(x, y),
            PointerKind::Hover => self.app.queue_hover_at(x, y),
            PointerKind::SecondaryClick => self.app.queue_secondary_click(x, y),
            PointerKind::Drag => {
                let (Some(to_x), Some(to_y)) = (args.to_x, args.to_y) else {
                    return ToolResult::error("`drag` requires `to_x` and `to_y`");
                };
                let anchor = args.to_node.or(args.node);
                let (to_x, to_y) = match self.anchor_point(anchor, to_x, to_y) {
                    Ok(point) => point,
                    Err(result) => return result,
                };
                let steps = args
                    .steps
                    .map_or_else(DragOptions::default, |steps| DragOptions {
                        steps,
                        ..DragOptions::default()
                    });
                self.app.queue_drag_from_to_with(x, y, to_x, to_y, steps);
            }
            PointerKind::Scroll => {
                self.app.queue_scroll_at(
                    x,
                    y,
                    args.dx.unwrap_or(0.0),
                    args.dy.unwrap_or(0.0),
                    matches!(args.unit, Some(ScrollUnit::Line)),
                );
            }
            PointerKind::Magnify => {
                let Some(factor) = args.factor else {
                    return ToolResult::error("`magnify` requires `factor`");
                };
                self.app.queue_magnify_at(x, y, factor);
            }
        }
        self.finish_input(args.settle)
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
        let key = if let (Some(ch), None) = (chars.next(), chars.next()) {
            KeyCode::Character(ch.to_string())
        } else {
            KeyCode::Named(args.key.clone())
        };
        self.app.queue_key_press(key, modifiers);
        self.finish_input(args.settle)
    }

    /// Answers an input dispatch: `settle` (the default) pumps the runtime to
    /// quiescence and returns the fresh tree; `settle: false` leaves the
    /// queued input's transient observable to `advance` and `screenshot`.
    fn finish_input(&mut self, settle: Option<bool>) -> ToolResult {
        if settle.unwrap_or(true) {
            self.app.settle();
            self.tree_text()
        } else {
            ToolResult::text(
                "queued (not settled); use `advance` or `screenshot` to observe the transient",
            )
        }
    }

    fn advance(&mut self, args: &AdvanceArgs) -> ToolResult {
        let duration = args
            .duration_ms
            .map_or(VIRTUAL_FRAME, Duration::from_millis);
        if args.screenshot.unwrap_or(false) {
            // The readback pump inside `snapshot` is the last frame of the
            // advance, so the image lands exactly `duration` past the previous
            // instant.
            self.app.pump_for(duration.saturating_sub(VIRTUAL_FRAME));
            return self.screenshot();
        }
        self.app.pump_for(duration);
        let status = if self.app.is_settled() {
            "settled"
        } else {
            "animating"
        };
        ToolResult::text(format!(
            "advanced {}ms; {status}\n\n{}",
            duration.as_millis(),
            tree::render_text(&self.app)
        ))
    }

    fn wait(&mut self, args: &WaitArgs) -> ToolResult {
        let mut expectations = Vec::new();
        if let Some(expect) = &args.exists {
            match selector_from(&mut self.app, &expect.selector) {
                Ok(selector) => expectations.push(apply_inverted(
                    self.app.expect_exists(selector),
                    expect.inverted,
                )),
                Err(error) => return ToolResult::error(error),
            }
        }
        if let Some(expect) = &args.not_exists {
            match selector_from(&mut self.app, &expect.selector) {
                Ok(selector) => expectations.push(apply_inverted(
                    self.app.expect_not_exists(selector),
                    expect.inverted,
                )),
                Err(error) => return ToolResult::error(error),
            }
        }
        if let Some(value_eq) = &args.value_eq {
            match selector_from(&mut self.app, &value_eq.selector) {
                Ok(selector) => expectations.push(apply_inverted(
                    self.app.expect_value_eq(selector, value_eq.value.clone()),
                    value_eq.inverted,
                )),
                Err(error) => return ToolResult::error(error),
            }
        }
        if let Some(expect) = &args.focus {
            match selector_from(&mut self.app, &expect.selector) {
                Ok(selector) => expectations.push(apply_inverted(
                    self.app.expect_ui_focus(selector),
                    expect.inverted,
                )),
                Err(error) => return ToolResult::error(error),
            }
        }
        if expectations.is_empty() {
            return ToolResult::error(
                "`wait` requires at least one of `exists`, `not_exists`, `value_eq`, or `focus`",
            );
        }
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(5000));
        let options = WaitOptions::new(timeout).enforce_order(args.enforce_order.unwrap_or(false));
        let status = match self.app.wait_for(&expectations, options) {
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

/// Applies a `wait` argument's `inverted` flag to a built expectation.
fn apply_inverted(
    expectation: waterui_testing::Expectation,
    inverted: Option<bool>,
) -> waterui_testing::Expectation {
    if inverted.unwrap_or(false) {
        expectation.inverted()
    } else {
        expectation
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
        ActAction::ScrollLeft => Ok((Action::ScrollLeft, None)),
        ActAction::ScrollRight => Ok((Action::ScrollRight, None)),
        ActAction::ScrollIntoView => Ok((Action::ScrollIntoView, None)),
        ActAction::ClearFocus => {
            unreachable!("clear_focus is dispatched before node resolution")
        }
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
