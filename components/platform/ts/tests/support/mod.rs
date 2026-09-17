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
use waterui_testing::{NodeId, OffscreenApp, SemanticApp, Snapshot, TreeSnapshot, ui};
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

    /// Mounts the module's view in an offscreen session, for a test whose
    /// observable is the rendered frame rather than the tree.
    ///
    /// # Panics
    ///
    /// Panics when the view has already been mounted.
    pub fn offscreen(&self) -> OffscreenApp {
        let view = self
            .view
            .borrow_mut()
            .take()
            .expect("a mounted module is realized once");
        mount_view_offscreen(view)
    }

    /// The bridge, for a test that watches what the mount exported.
    pub const fn bridge(&self) -> &waterui_ts::Bridge {
        self.runtime.bridge()
    }

    /// The accessibility tree the module's view produces.
    pub fn tree(&self) -> String {
        render(self.app().tree())
    }

    /// The frame the module's view renders.
    pub fn pixels(&self) -> Snapshot {
        self.offscreen().snapshot()
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

/// Mounts one view in an offscreen session.
///
/// The same viewport as [`mount_view`], so a frame and a tree of one view
/// describe the same layout.
pub fn mount_view_offscreen(view: AnyView) -> OffscreenApp {
    let view = RefCell::new(Some(view));
    ui().viewport(320, 240).mount_offscreen(move || {
        view.borrow_mut()
            .take()
            .expect("the test session realizes its root once")
    })
}

/// The accessibility tree of a Rust view.
pub fn rust_tree(view: impl waterui_core::View + 'static) -> String {
    render(mount_view(AnyView::new(view)).tree())
}

/// The frame a Rust view renders.
pub fn rust_pixels(view: impl waterui_core::View + 'static) -> Snapshot {
    mount_view_offscreen(AnyView::new(view)).snapshot()
}

/// Asserts two frames are the same picture.
///
/// This is the equivalence a purely visual modifier — a colour, an opacity, a
/// shadow, a clip — leaves to check: it moves no node, label, action or
/// bound, so the accessibility tree is the same tree whether the attribute
/// was read or dropped, and only the frame can tell. Both frames come from
/// the same renderer on the same adapter in the same process, drawing what
/// is meant to be the same scene, so they are compared exactly rather than
/// judged: the Rust form is the reference, and a JSX form that drops the
/// attribute draws a different picture.
///
/// # Panics
///
/// Panics when the frames differ in size or in any pixel.
pub fn assert_same_pixels(case: &str, jsx: &Snapshot, rust: &Snapshot) {
    assert_eq!(
        (jsx.width, jsx.height),
        (rust.width, rust.height),
        "{case}: the JSX form and the Rust form render at different sizes"
    );
    let (jsx_pixels, _) = jsx.rgba8.as_chunks::<4>();
    let (rust_pixels, _) = rust.rgba8.as_chunks::<4>();
    let differing = jsx_pixels
        .iter()
        .zip(rust_pixels)
        .filter(|(left, right)| left != right)
        .count();
    assert_eq!(
        differing,
        0,
        "{case}: the JSX form and the Rust form differ in {differing} of {} pixels",
        jsx_pixels.len()
    );
}

/// The tree as text: one line per node, indented by depth.
///
/// Node ids are left out because they are assigned per mount and would never
/// match between two sessions; everything that describes what the node *is* —
/// its role, what it says, its identifier and states, what it can do, and
/// where it sits — is in.
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
        "{indent}{:?} label={:?} id={:?} value={:?} checked={:?} selected={} expanded={:?} \
         busy={} enabled={} hidden={} actions=[{actions}] {bounds}",
        node.role(),
        node.label(),
        node.identifier(),
        node.value(),
        node.checked(),
        node.selected(),
        node.expanded(),
        node.busy(),
        node.enabled(),
        node.hidden(),
    )
    .expect("writing to a string cannot fail");
    for child in node.children() {
        write_node(rendered, tree, *child, depth + 1);
    }
}
