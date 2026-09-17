//! Mounting a TypeScript module and a Rust view, and comparing what each one
//! puts in the accessibility tree.
//!
//! The JavaScript side is the real library — `fixtures/library.js`, bundled
//! from `src/js` by `scripts/build-test-library.sh` — loaded into the engine
//! this target selects, with the host table under test installed. What a test
//! writes is what the reactive JSX transform emits: `jsx(name, props)` with
//! the children in a `children` property, so the calls the host receives are
//! the calls an application makes.

use core::cell::RefCell;
use std::fmt::Write as _;

use nami::Computed;
use waterui::ts::Components;
use waterui_core::{AnyView, Environment};
use waterui_graphics::color::ColorScheme;
use waterui_testing::{NodeId, SemanticApp, TreeSnapshot, ui};
use waterui_ts::engine::{JsRuntime as _, JsValue};
use waterui_ts::{MountScope, TsRuntime, ViewSlot};

/// The `waterui` JavaScript library, as a classic script.
const LIBRARY: &str = include_str!("../fixtures/library.js");

/// One mounted TypeScript module: the runtime, the scope that owns what it
/// exported, and the view it produced.
///
/// The runtime outlives the view on purpose — a materialized signal calls back
/// into the engine — so the whole thing is dropped together, after the tree
/// has been read.
pub struct Module {
    runtime: TsRuntime,
    scope: Option<MountScope>,
    view: RefCell<Option<AnyView>>,
}

impl Module {
    /// Evaluates `source` after the library and mounts the module it
    /// published as `main`.
    ///
    /// # Panics
    ///
    /// Panics when the bundle throws, when it publishes no `main`, or when
    /// mounting it does not produce a view.
    pub fn mount(source: &str) -> Self {
        let environment = Environment::new()
            .store::<ColorScheme, Computed<ColorScheme>>(Computed::constant(ColorScheme::Light));
        let runtime = TsRuntime::new(environment, Components).expect("the runtime constructs");
        runtime
            .load(&format!("{LIBRARY}\n{source}"))
            .unwrap_or_else(|error| panic!("loading the bundle: {error}"));
        let module = runtime
            .module("main")
            .expect("the bundle publishes a `main` module");
        let scope = runtime.bridge().open_scope();
        let mounted = runtime
            .bridge()
            .call(
                runtime.runtime().expect("a bundle is loaded").mount(),
                &[JsValue::Function(module)],
            )
            .unwrap_or_else(|error| panic!("mounting the module: {error}"));
        let view = handle_of(&mounted);
        Self {
            runtime,
            scope: Some(scope),
            view: RefCell::new(Some(view)),
        }
    }

    /// Evaluates an expression in the module's engine.
    ///
    /// # Panics
    ///
    /// Panics when the expression throws.
    pub fn eval(&self, source: &str) -> JsValue {
        self.runtime
            .bridge()
            .engine()
            .eval(source, "test.js")
            .unwrap_or_else(|error| panic!("evaluating `{source}`: {error}"))
    }

    /// Mounts the module's view in a test session and hands back the session,
    /// which borrows nothing: the runtime stays alive in `self`.
    ///
    /// # Panics
    ///
    /// Panics when the view has already been mounted.
    pub fn app(&self) -> SemanticApp {
        let view = self
            .view
            .borrow_mut()
            .take()
            .expect("a mounted module is realized once");
        mount_view(view)
    }

    /// The bridge, for a test that watches what the mount exported.
    pub const fn bridge(&self) -> &waterui_ts::Bridge {
        self.runtime.bridge()
    }

    /// The accessibility tree the module's view produces.
    pub fn tree(&self) -> String {
        render(self.app().tree())
    }
}

/// Mounts `source` and hands back the error it was refused with.
///
/// # Panics
///
/// Panics when the module mounts, which is the failure this reports on.
pub fn mount_error(source: &str) -> String {
    let environment = Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::constant(ColorScheme::Light));
    let runtime = TsRuntime::new(environment, Components).expect("the runtime constructs");
    runtime
        .load(&format!("{LIBRARY}\n{source}"))
        .unwrap_or_else(|error| panic!("loading the bundle: {error}"));
    let module = runtime
        .module("main")
        .expect("the bundle publishes a `main` module");
    let _scope = runtime.bridge().open_scope();
    let error = runtime
        .bridge()
        .call(
            runtime.runtime().expect("a bundle is loaded").mount(),
            &[JsValue::Function(module)],
        )
        .expect_err("the module was expected to be refused");
    error.to_string()
}

impl Drop for Module {
    /// Releases what the mount exported before the runtime goes.
    fn drop(&mut self) {
        self.scope = None;
    }
}

/// The view a `{ handle, dispose }` carries.
fn handle_of(mounted: &JsValue) -> AnyView {
    let entries = mounted
        .as_object()
        .unwrap_or_else(|| panic!("mount() returned {mounted:?}, not a mounted tree"));
    let handle = entries
        .iter()
        .find(|(key, _)| key == "handle")
        .map(|(_, value)| value)
        .expect("a mounted tree carries its view under `handle`");
    ViewSlot::from_js_value(handle)
        .expect("the handle is a view slot")
        .take()
        .expect("the view has not been taken")
}

/// Mounts one view in a test session.
///
/// `mount` takes a builder, and a view crosses from TypeScript once, so the
/// builder hands the view over exactly once and says so if it is asked twice.
pub fn mount_view(view: AnyView) -> SemanticApp {
    let view = RefCell::new(Some(view));
    ui().viewport(320, 240).mount(move || {
        view.borrow_mut()
            .take()
            .expect("the test session realizes its root once")
    })
}

/// The accessibility tree of a Rust view.
pub fn rust_tree(view: impl waterui_core::View + 'static) -> String {
    render(mount_view(AnyView::new(view)).tree())
}

/// The tree as text: one line per node, indented by depth.
///
/// Identifiers are left out because they are assigned per mount and would
/// never match between two sessions; everything that describes what the node
/// *is* — its role, what it says, what it can do, and where it sits — is in.
pub fn render(tree: &TreeSnapshot) -> String {
    let mut rendered = String::new();
    write_node(&mut rendered, tree, tree.root(), 0);
    rendered
}

/// One node and everything below it.
fn write_node(rendered: &mut String, tree: &TreeSnapshot, id: NodeId, depth: usize) {
    let Some(node) = tree.node(id) else {
        return;
    };
    let indent = "  ".repeat(depth);
    let bounds = node.bounds().map_or_else(
        || String::from("unplaced"),
        |bounds| {
            format!(
                "{:.1}×{:.1} at {:.1},{:.1}",
                bounds.width(),
                bounds.height(),
                bounds.x(),
                bounds.y()
            )
        },
    );
    let actions = node
        .actions()
        .iter()
        .map(|action| format!("{action:?}"))
        .collect::<Vec<_>>()
        .join("+");
    writeln!(
        rendered,
        "{indent}{:?} label={:?} value={:?} checked={:?} enabled={} hidden={} actions=[{actions}] \
         {bounds}",
        node.role(),
        node.label(),
        node.value(),
        node.checked(),
        node.enabled(),
        node.hidden(),
    )
    .expect("writing to a string cannot fail");
    for child in node.children() {
        write_node(rendered, tree, *child, depth + 1);
    }
}
