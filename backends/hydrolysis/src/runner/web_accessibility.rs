use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

use accesskit::{Action, ActionRequest, Node, NodeId, Point, Role, Toggled, TreeId, TreeUpdate};
use wasm_bindgen::{JsCast, closure::Closure};
use waterui_core::contract_violation;
use web_sys::{Event, HtmlElement};

struct DomNode {
    element: HtmlElement,
    _click: Closure<dyn FnMut(Event)>,
    _focus: Closure<dyn FnMut(Event)>,
}

pub(super) struct WebAccessibilityBridge {
    root: HtmlElement,
    nodes: HashMap<NodeId, DomNode>,
    actions: Rc<RefCell<VecDeque<ActionRequest>>>,
    schedule_frame: Rc<dyn Fn()>,
}

/// The `result` of a DOM call, or `None` after logging that `step` failed:
/// the mirror skips what the browser refused.
fn dom<T, E: std::fmt::Debug>(step: &str, result: Result<T, E>) -> Option<T> {
    result
        .inspect_err(|error| {
            tracing::error!(
                "hydrolysis web accessibility: failed to {step}: {error:?}; the mirror skips it"
            );
        })
        .ok()
}

impl WebAccessibilityBridge {
    /// The bridge mounted on the page's body, or `None` after logging when
    /// the page offers no window, document or body to mount on: the page then
    /// has no accessibility mirror.
    pub(super) fn new(
        actions: Rc<RefCell<VecDeque<ActionRequest>>>,
        schedule_frame: Rc<dyn Fn()>,
    ) -> Option<Self> {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            tracing::error!(
                "hydrolysis web accessibility: the page has no browser window or document; the page has no accessibility mirror"
            );
            return None;
        };
        let root = dom(
            "create the accessibility root",
            document.create_element("div"),
        )?;
        let root = dom(
            "treat the accessibility root as an HtmlElement",
            root.dyn_into::<HtmlElement>(),
        )?;
        root.set_id("waterui-accessibility");
        let style = root.style();
        // Absolute, not fixed: Safari treats a viewport-sized fixed element
        // as the page's fixed bar and tints its status-bar and toolbar bands
        // from it, which paints them black over the GPU canvas.
        for (name, value) in [
            ("position", "absolute"),
            ("inset", "0"),
            ("pointer-events", "none"),
            ("overflow", "hidden"),
            ("color", "transparent"),
            ("background", "transparent"),
        ] {
            dom(
                "set a web accessibility root style",
                style.set_property(name, value),
            );
        }
        let Some(body) = document.body() else {
            tracing::error!(
                "hydrolysis web accessibility: the document has no body; the page has no accessibility mirror"
            );
            return None;
        };
        dom("append the accessibility root", body.append_child(&root))?;
        Some(Self {
            root,
            nodes: HashMap::new(),
            actions,
            schedule_frame,
        })
    }

    pub(super) fn update(&mut self, update: TreeUpdate) {
        let active: HashSet<_> = update.nodes.iter().map(|(id, _)| *id).collect();
        self.nodes.retain(|id, node| {
            if active.contains(id) {
                true
            } else {
                node.element.remove();
                false
            }
        });
        let accesskit_nodes: HashMap<_, _> =
            update.nodes.iter().map(|(id, node)| (*id, node)).collect();
        let parents: HashMap<_, _> = update
            .nodes
            .iter()
            .flat_map(|(id, node)| node.children().iter().map(move |child| (*child, *id)))
            .collect();
        for (id, node) in &update.nodes {
            self.ensure_node(*id);
            // A node the browser refused to create stays out of the mirror.
            let Some(element) = self.nodes.get(id).map(|node| node.element.clone()) else {
                continue;
            };
            apply_node(
                &element,
                node,
                positioned_ancestor_origin(*id, &parents, &accesskit_nodes),
            );
        }
        for (id, node) in &update.nodes {
            let Some(parent) = self.nodes.get(id).map(|node| node.element.clone()) else {
                continue;
            };
            for child_id in node.children() {
                let Some(child) = self.nodes.get(child_id) else {
                    contract_violation!(
                        "accessibility child {child_id:?} missing from full tree update; the child stays out of the mirror"
                    );
                    continue;
                };
                dom(
                    "attach a DOM accessibility child",
                    parent.append_child(&child.element),
                );
            }
        }
        let Some(tree) = update.tree else {
            contract_violation!(
                "web accessibility requires a full tree update; the mirror's tree is not attached"
            );
            return;
        };
        let Some(root) = self.nodes.get(&tree.root) else {
            contract_violation!(
                "web accessibility root node is missing; the mirror's tree is not attached"
            );
            return;
        };
        dom(
            "attach the DOM accessibility tree",
            self.root.append_child(&root.element),
        );
        if let Some(focused) = self.nodes.get(&update.focus)
            && let Some(document) = self.root.owner_document()
        {
            let active = document.active_element();
            let already_focused = active.as_ref() == Some(focused.element.as_ref());
            // DOM focus follows Hydrolysis focus only while it already lives
            // in this tree, as it does for a screen-reader user. Otherwise it
            // sits on the runtime's input surfaces — the canvas for keys, the
            // hidden input for text — and moving it here would take keyboard
            // and text input away from the app.
            let focus_in_tree = active
                .as_ref()
                .is_some_and(|active| self.root.contains(Some(active.as_ref())));
            if focus_in_tree && !already_focused && focused.element.tab_index() >= 0 {
                dom("focus a DOM accessibility node", focused.element.focus());
            }
        }
    }

    /// Creates the DOM node for `id` when it has none; a failing browser
    /// call logs and leaves the node out of the mirror.
    fn ensure_node(&mut self, id: NodeId) {
        if self.nodes.contains_key(&id) {
            return;
        }
        let Some(document) = self.root.owner_document() else {
            tracing::error!(
                "hydrolysis web accessibility: the accessibility root has no document; the node stays out of the mirror"
            );
            return;
        };
        let Some(element) = dom(
            "create a DOM accessibility node",
            document.create_element("div"),
        ) else {
            return;
        };
        let Some(element) = dom(
            "treat a DOM accessibility node as an HtmlElement",
            element.dyn_into::<HtmlElement>(),
        ) else {
            return;
        };
        element.style().set_css_text(
            // Hydrolysis draws its own focus ring; the mirror draws none.
            "position:absolute;pointer-events:none;color:transparent;background:transparent;outline:none;",
        );

        let click_actions = Rc::clone(&self.actions);
        let click_schedule = Rc::clone(&self.schedule_frame);
        let click = Closure::wrap(Box::new(move |event: Event| {
            event.prevent_default();
            click_actions.borrow_mut().push_back(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: id,
                data: None,
            });
            click_schedule();
        }) as Box<dyn FnMut(Event)>);
        dom(
            "install the DOM accessibility click listener",
            element.add_event_listener_with_callback("click", click.as_ref().unchecked_ref()),
        );

        let focus_actions = Rc::clone(&self.actions);
        let focus_schedule = Rc::clone(&self.schedule_frame);
        let focus = Closure::wrap(Box::new(move |_event: Event| {
            focus_actions.borrow_mut().push_back(ActionRequest {
                action: Action::Focus,
                target_tree: TreeId::ROOT,
                target_node: id,
                data: None,
            });
            focus_schedule();
        }) as Box<dyn FnMut(Event)>);
        dom(
            "install the DOM accessibility focus listener",
            element.add_event_listener_with_callback("focus", focus.as_ref().unchecked_ref()),
        );
        self.nodes.insert(
            id,
            DomNode {
                element,
                _click: click,
                _focus: focus,
            },
        );
    }
}

fn set_optional_attribute(element: &HtmlElement, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        dom(
            "set a DOM accessibility attribute",
            element.set_attribute(name, value),
        );
    } else {
        dom(
            "remove a DOM accessibility attribute",
            element.remove_attribute(name),
        );
    }
}

/// Window position of the nearest ancestor that carries bounds.
///
/// AccessKit bounds are window coordinates, while each mirror node is
/// positioned inside its parent's element; an ancestor without bounds sits at
/// its own parent's origin, so the offset comes from the nearest one with
/// bounds.
fn positioned_ancestor_origin(
    id: NodeId,
    parents: &HashMap<NodeId, NodeId>,
    nodes: &HashMap<NodeId, &Node>,
) -> Point {
    let mut current = id;
    while let Some(parent) = parents.get(&current) {
        if let Some(bounds) = nodes[parent].bounds() {
            return bounds.origin();
        }
        current = *parent;
    }
    Point::ZERO
}

fn apply_node(element: &HtmlElement, node: &Node, origin: Point) {
    let is_text = matches!(node.role(), Role::TextRun | Role::Label | Role::Paragraph);
    element.set_text_content(is_text.then(|| node.label()).flatten());
    set_optional_attribute(element, "role", aria_role(node.role()));
    set_optional_attribute(
        element,
        "aria-label",
        (!is_text).then(|| node.label()).flatten(),
    );
    set_optional_attribute(element, "aria-description", node.description());
    set_optional_attribute(
        element,
        "aria-disabled",
        node.is_disabled().then_some("true"),
    );
    set_optional_attribute(
        element,
        "aria-selected",
        node.is_selected()
            .map(|value| if value { "true" } else { "false" }),
    );
    set_optional_attribute(
        element,
        "aria-expanded",
        node.is_expanded()
            .map(|value| if value { "true" } else { "false" }),
    );
    set_optional_attribute(element, "aria-busy", node.is_busy().then_some("true"));
    set_optional_attribute(element, "aria-hidden", node.is_hidden().then_some("true"));
    set_optional_attribute(
        element,
        "aria-checked",
        node.toggled().map(|value| match value {
            Toggled::False => "false",
            Toggled::True => "true",
            Toggled::Mixed => "mixed",
        }),
    );
    set_optional_attribute(
        element,
        "aria-valuenow",
        node.numeric_value()
            .map(|value| value.to_string())
            .as_deref(),
    );
    set_optional_attribute(
        element,
        "aria-valuemin",
        node.min_numeric_value()
            .map(|value| value.to_string())
            .as_deref(),
    );
    set_optional_attribute(
        element,
        "aria-valuemax",
        node.max_numeric_value()
            .map(|value| value.to_string())
            .as_deref(),
    );
    set_optional_attribute(element, "aria-valuetext", node.value());
    set_optional_attribute(
        element,
        "aria-level",
        (node.role() == Role::Heading)
            .then(|| node.level())
            .flatten()
            .map(|level| level.to_string())
            .as_deref(),
    );
    set_optional_attribute(element, "data-waterui-id", node.author_id());
    element.set_tab_index(
        if node.supports_action(Action::Focus) || node.supports_action(Action::Click) {
            0
        } else {
            -1
        },
    );
    if let Some(bounds) = node.bounds() {
        let style = element.style();
        for (name, value) in [
            ("left", format!("{}px", bounds.x0 - origin.x)),
            ("top", format!("{}px", bounds.y0 - origin.y)),
            ("width", format!("{}px", bounds.width().max(0.0))),
            ("height", format!("{}px", bounds.height().max(0.0))),
        ] {
            dom(
                "set DOM accessibility bounds",
                style.set_property(name, &value),
            );
        }
    }
}

const fn aria_role(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Unknown
        | Role::GenericContainer
        | Role::Window
        | Role::TextRun
        | Role::Label
        | Role::Paragraph => return None,
        Role::Cell | Role::LayoutTableCell | Role::GridCell => "cell",
        Role::Row | Role::LayoutTableRow => "row",
        Role::RowHeader => "rowheader",
        Role::ColumnHeader => "columnheader",
        Role::Image => "img",
        Role::Link => "link",
        Role::ListItem => "listitem",
        Role::ListBoxOption | Role::MenuListOption => "option",
        Role::CheckBox => "checkbox",
        Role::RadioButton => "radio",
        Role::Button | Role::DefaultButton => "button",
        Role::List => "list",
        Role::Table | Role::LayoutTable => "table",
        Role::Switch => "switch",
        Role::Menu => "menu",
        Role::SearchInput | Role::Search => "search",
        Role::Article => "article",
        Role::Footer => "contentinfo",
        Role::Header => "banner",
        Role::ComboBox | Role::EditableComboBox => "combobox",
        Role::Heading => "heading",
        Role::Main => "main",
        Role::MenuBar => "menubar",
        Role::MenuItem => "menuitem",
        Role::MenuItemCheckBox => "menuitemcheckbox",
        Role::MenuItemRadio => "menuitemradio",
        Role::Navigation => "navigation",
        Role::ProgressIndicator => "progressbar",
        Role::Slider => "slider",
        Role::Tab => "tab",
        Role::TabList => "tablist",
        Role::TabPanel => "tabpanel",
        Role::TextInput | Role::MultilineTextInput | Role::PasswordInput => "textbox",
        Role::SpinButton => "spinbutton",
        Role::ScrollView | Role::Section | Role::Region => "region",
        Role::RootWebArea | Role::Application => "application",
        _ => "group",
    })
}
