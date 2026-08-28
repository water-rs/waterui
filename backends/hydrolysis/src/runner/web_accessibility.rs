use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, TreeId, TreeUpdate};
use wasm_bindgen::{JsCast, closure::Closure};
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

impl WebAccessibilityBridge {
    pub(super) fn new(
        actions: Rc<RefCell<VecDeque<ActionRequest>>>,
        schedule_frame: Rc<dyn Fn()>,
    ) -> Self {
        let document = web_sys::window()
            .expect("web accessibility requires a browser window")
            .document()
            .expect("web accessibility requires a document");
        let root = document
            .create_element("div")
            .expect("failed to create web accessibility root")
            .dyn_into::<HtmlElement>()
            .expect("web accessibility root is not an HtmlElement");
        root.set_id("waterui-accessibility");
        let style = root.style();
        for (name, value) in [
            ("position", "fixed"),
            ("inset", "0"),
            ("pointer-events", "none"),
            ("overflow", "hidden"),
            ("color", "transparent"),
            ("background", "transparent"),
        ] {
            style.set_property(name, value).unwrap_or_else(|error| {
                panic!("failed to set web accessibility root style {name}: {error:?}")
            });
        }
        document
            .body()
            .expect("web accessibility requires a document body")
            .append_child(&root)
            .expect("failed to append web accessibility root");
        Self {
            root,
            nodes: HashMap::new(),
            actions,
            schedule_frame,
        }
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
        for (id, node) in &update.nodes {
            self.ensure_node(*id);
            let element = self
                .nodes
                .get(id)
                .expect("new DOM accessibility node missing")
                .element
                .clone();
            apply_node(&element, node);
        }
        let accesskit_nodes: HashMap<_, _> =
            update.nodes.iter().map(|(id, node)| (*id, node)).collect();
        for (id, node) in &update.nodes {
            let parent = self
                .nodes
                .get(id)
                .expect("DOM accessibility parent missing")
                .element
                .clone();
            for child_id in node.children() {
                let child = self.nodes.get(child_id).unwrap_or_else(|| {
                    panic!("accessibility child {child_id:?} missing from full tree update")
                });
                parent
                    .append_child(&child.element)
                    .expect("failed to attach DOM accessibility child");
            }
        }
        let tree = update
            .tree
            .expect("web accessibility requires a full tree update");
        assert!(
            accesskit_nodes.contains_key(&tree.root),
            "web accessibility root node is missing"
        );
        self.root
            .append_child(
                &self
                    .nodes
                    .get(&tree.root)
                    .expect("DOM root missing")
                    .element,
            )
            .expect("failed to attach DOM accessibility tree");
        if let Some(focused) = self.nodes.get(&update.focus) {
            let document = self
                .root
                .owner_document()
                .expect("accessibility root has no document");
            let already_focused =
                document.active_element().as_ref() == Some(focused.element.as_ref());
            if !already_focused && focused.element.tab_index() >= 0 {
                focused
                    .element
                    .focus()
                    .expect("failed to focus DOM accessibility node");
            }
        }
    }

    fn ensure_node(&mut self, id: NodeId) {
        if self.nodes.contains_key(&id) {
            return;
        }
        let document = self
            .root
            .owner_document()
            .expect("accessibility root has no document");
        let element = document
            .create_element("div")
            .expect("failed to create DOM accessibility node")
            .dyn_into::<HtmlElement>()
            .expect("DOM accessibility node is not an HtmlElement");
        element.style().set_css_text(
            "position:absolute;pointer-events:none;color:transparent;background:transparent;",
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
        element
            .add_event_listener_with_callback("click", click.as_ref().unchecked_ref())
            .expect("failed to install DOM accessibility click listener");

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
        element
            .add_event_listener_with_callback("focus", focus.as_ref().unchecked_ref())
            .expect("failed to install DOM accessibility focus listener");
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
        element.set_attribute(name, value).unwrap_or_else(|error| {
            panic!("failed to set DOM accessibility attribute {name}: {error:?}")
        });
    } else {
        element.remove_attribute(name).unwrap_or_else(|error| {
            panic!("failed to remove DOM accessibility attribute {name}: {error:?}")
        });
    }
}

fn apply_node(element: &HtmlElement, node: &Node) {
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
            ("left", format!("{}px", bounds.x0)),
            ("top", format!("{}px", bounds.y0)),
            ("width", format!("{}px", bounds.width().max(0.0))),
            ("height", format!("{}px", bounds.height().max(0.0))),
        ] {
            style.set_property(name, &value).unwrap_or_else(|error| {
                panic!("failed to set DOM accessibility bounds {name}: {error:?}")
            });
        }
    }
}

fn aria_role(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Unknown | Role::GenericContainer | Role::Window => return None,
        Role::TextRun | Role::Label | Role::Paragraph => return None,
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
        Role::Group | Role::Pane => "group",
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
        Role::ScrollView => "region",
        Role::RootWebArea | Role::Application => "application",
        Role::Section | Role::Region => "region",
        _ => "group",
    })
}
