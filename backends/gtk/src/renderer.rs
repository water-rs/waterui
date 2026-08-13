//! View renderer that dispatches `WaterUI` views to GTK widgets.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glib::object::ObjectExt;
use glib::value::ToValue;
use gtk4::Widget;
use gtk4::prelude::*;
use gtk4::{Align, Overflow};
use nami::Signal;
use waterui::accessibility::{
    AccessibilityChecked, AccessibilityChildren, AccessibilityHidden, AccessibilityLabel,
    AccessibilityRole, AccessibilityState, AccessibilityStateSignal,
};
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::list::ListConfig;
use waterui::component::progress::ProgressConfig;
use waterui::cursor::{Cursor, CursorStyle};
use waterui::drag_drop::{DragData, Draggable, DropDestination};
use waterui::interaction::Hittable;
use waterui::metadata::context_menu::ResolvedContextMenu;
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::prelude::Divider;
use waterui::style::{Offset, Rotation, Scale};
use waterui_backend_core::ViewDispatcher;
#[cfg(feature = "chromium")]
use waterui_chromium::ChromiumView;
use waterui_controls::button::ButtonConfig;
use waterui_controls::menu::ResolvedMenu;
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::ResolvedTextFieldConfig;
use waterui_controls::toggle::ToggleConfig;
use waterui_core::Binding;
use waterui_core::dynamic::Dynamic;
use waterui_core::event::{Event, HoverEvent, LifeCycle, LifeCycleHook, OnEvent};
use waterui_core::handler::BoxedAction;
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Environment, Native, View};
use waterui_core::{IgnorableMetadata, Metadata, Retain, Str};
use waterui_form::picker::PickerConfig;
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::date::DatePickerConfig;
use waterui_form::picker::multi_date::MultiDatePickerConfig;
use waterui_form::secure::SecureFieldConfig;
use waterui_graphics::gpu_surface::GpuSurface;
use waterui_graphics::{
    AppliedFilter, ResolvedGradient,
    color::{Color, ResolvedColor},
};
use waterui_icon::SystemIcon;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_navigation::tab::Tabs;
use waterui_navigation::{
    NavigationSplitLayout, NavigationStack, NavigationTransitionDestination,
    NavigationTransitionSource, NavigationView,
};
use waterui_shape::{ClipShape, PathCommand, ResolvedShape};
use waterui_text::TextConfig;
#[cfg(any(
    feature = "webview-default",
    feature = "webview-system",
    feature = "webview-wpe",
    feature = "webview-cef"
))]
use waterui_webview::WebView;

use crate::component::GtkComponent;
use crate::components::menu::rebuild_menu_popover;
use crate::util::{ScopedCss, store_watcher_guard, subscribe_then_get};

const FOCUS_ANCHOR_DATA_KEY: &str = "waterui_focus_anchor";
const FOCUS_REQUEST_PENDING_DATA_KEY: &str = "waterui_focus_request_pending";
const FOCUS_MAP_HANDLER_INSTALLED_DATA_KEY: &str = "waterui_focus_map_handler_installed";
const RETAIN_DATA_KEY: &str = "waterui-retain-metadata";

fn gtk_accessibility_role(role: AccessibilityRole) -> gtk4::AccessibleRole {
    match role {
        AccessibilityRole::Button => gtk4::AccessibleRole::Button,
        AccessibilityRole::Link => gtk4::AccessibleRole::Link,
        AccessibilityRole::Image => gtk4::AccessibleRole::Img,
        AccessibilityRole::Text => gtk4::AccessibleRole::Label,
        AccessibilityRole::Header => gtk4::AccessibleRole::Heading,
        AccessibilityRole::Footer => gtk4::AccessibleRole::Section,
        AccessibilityRole::Navigation => gtk4::AccessibleRole::Navigation,
        AccessibilityRole::Main => gtk4::AccessibleRole::Main,
        AccessibilityRole::Search => gtk4::AccessibleRole::Search,
        AccessibilityRole::Article => gtk4::AccessibleRole::Document,
        AccessibilityRole::Section => gtk4::AccessibleRole::Section,
        AccessibilityRole::List => gtk4::AccessibleRole::List,
        AccessibilityRole::ListItem => gtk4::AccessibleRole::ListItem,
        AccessibilityRole::Checkbox => gtk4::AccessibleRole::Checkbox,
        AccessibilityRole::RadioButton => gtk4::AccessibleRole::Radio,
        AccessibilityRole::Switch => gtk4::AccessibleRole::Switch,
        AccessibilityRole::Slider => gtk4::AccessibleRole::Slider,
        AccessibilityRole::ProgressBar => gtk4::AccessibleRole::ProgressBar,
        AccessibilityRole::Tab => gtk4::AccessibleRole::Tab,
        AccessibilityRole::TabList => gtk4::AccessibleRole::TabList,
        AccessibilityRole::TabPanel => gtk4::AccessibleRole::TabPanel,
        AccessibilityRole::Menu => gtk4::AccessibleRole::Menu,
        AccessibilityRole::MenuItem => gtk4::AccessibleRole::MenuItem,
        AccessibilityRole::MenuBar => gtk4::AccessibleRole::MenuBar,
        AccessibilityRole::MenuItemCheckbox => gtk4::AccessibleRole::MenuItemCheckbox,
        AccessibilityRole::MenuItemRadio => gtk4::AccessibleRole::MenuItemRadio,
        AccessibilityRole::Combobox => gtk4::AccessibleRole::ComboBox,
        AccessibilityRole::Option => gtk4::AccessibleRole::Option,
        AccessibilityRole::Group => gtk4::AccessibleRole::Group,
        _ => panic!("unsupported accessibility role in GTK backend"),
    }
}

fn apply_gtk_accessibility_state(widget: &Widget, state: &AccessibilityState) {
    let checked = match state.checked_state() {
        None => gtk4::AccessibleTristate::Undefined,
        Some(AccessibilityChecked::False) => gtk4::AccessibleTristate::False,
        Some(AccessibilityChecked::True) => gtk4::AccessibleTristate::True,
        Some(AccessibilityChecked::Mixed) => gtk4::AccessibleTristate::Mixed,
    };
    widget.update_state(&[
        gtk4::accessible::State::Disabled(state.is_disabled()),
        gtk4::accessible::State::Selected(Some(state.is_selected())),
        gtk4::accessible::State::Checked(checked),
        gtk4::accessible::State::Expanded(state.expanded_state()),
        gtk4::accessible::State::Busy(state.is_busy()),
        gtk4::accessible::State::Hidden(state.is_hidden()),
    ]);
}

#[derive(Debug)]
pub(crate) struct FocusAnchorMarker;

#[derive(Debug)]
struct PendingFocusRequest;

#[derive(Debug)]
struct FocusMapHandlerInstalled;

#[derive(Debug, Default)]
struct ReactiveAxisPair {
    x: Cell<Option<f32>>,
    y: Cell<Option<f32>>,
}

impl ReactiveAxisPair {
    fn update_x(&self, value: f32) {
        self.x.set(Some(value));
    }

    fn update_y(&self, value: f32) {
        self.y.set(Some(value));
    }

    fn values(&self) -> Option<(f32, f32)> {
        Some((self.x.get()?, self.y.get()?))
    }
}

pub(crate) fn mark_focus_anchor(widget: &impl IsA<Widget>) {
    // SAFETY: `FOCUS_ANCHOR_DATA_KEY` is private to this module and is only
    // ever paired with `FocusAnchorMarker`, so every read of the key downcasts
    // to the type stored here; widget data is only touched on the GTK main
    // thread.
    unsafe {
        widget
            .as_ref()
            .set_data(FOCUS_ANCHOR_DATA_KEY, FocusAnchorMarker);
    }
}

fn attach_focus_metadata(widget: Widget, binding: &Binding<bool>) -> Widget {
    let anchor = resolve_single_focus_anchor(&widget);

    anchor.connect_has_focus_notify({
        let binding = binding.clone();
        move |anchor| {
            let has_focus = anchor.has_focus();
            if binding.get() != has_focus {
                binding.set(has_focus);
            }
        }
    });

    let (focused, guard) = subscribe_then_get(binding, {
        let anchor = anchor.clone();
        move |ctx| {
            if ctx.into_value() {
                request_focus(&anchor);
            } else {
                clear_focus(&anchor);
            }
        }
    });
    if focused {
        request_focus(&anchor);
    }
    store_watcher_guard(&widget, guard);

    widget
}

fn resolve_single_focus_anchor(widget: &Widget) -> Widget {
    let mut anchors = Vec::new();
    collect_focus_anchors(widget, &mut anchors);
    assert!(
        anchors.len() == 1,
        "GTK Focused metadata requires exactly one TextField or SecureField anchor in its subtree, found {}",
        anchors.len()
    );
    anchors
        .pop()
        .expect("focus anchor count was asserted to be exactly one")
}

fn collect_focus_anchors(widget: &Widget, anchors: &mut Vec<Widget>) {
    // SAFETY: `FOCUS_ANCHOR_DATA_KEY` only ever stores `FocusAnchorMarker`
    // (see `mark_focus_anchor`), and the returned pointer is discarded after
    // the presence check, so no reference outlives this main-thread call.
    if unsafe { widget.data::<FocusAnchorMarker>(FOCUS_ANCHOR_DATA_KEY) }.is_some() {
        anchors.push(widget.clone());
    }

    let mut child = widget.first_child();
    while let Some(current) = child {
        collect_focus_anchors(&current, anchors);
        child = current.next_sibling();
    }
}

fn request_focus(anchor: &Widget) {
    if anchor.has_focus() {
        return;
    }

    if anchor.is_mapped() {
        assert!(
            anchor.grab_focus(),
            "GTK Focused metadata failed to focus its resolved TextField/SecureField anchor"
        );
        return;
    }

    // SAFETY: `FOCUS_REQUEST_PENDING_DATA_KEY` only ever stores
    // `PendingFocusRequest`, and the returned pointer is discarded after the
    // presence check, so no reference outlives this main-thread call.
    if unsafe { anchor.data::<PendingFocusRequest>(FOCUS_REQUEST_PENDING_DATA_KEY) }.is_some() {
        return;
    }

    // SAFETY: same key/type pairing as the check above; widget data is only
    // touched on the GTK main thread.
    unsafe { anchor.set_data(FOCUS_REQUEST_PENDING_DATA_KEY, PendingFocusRequest) };
    // SAFETY: `FOCUS_MAP_HANDLER_INSTALLED_DATA_KEY` only ever stores
    // `FocusMapHandlerInstalled`, and the returned pointer is discarded after
    // the presence check, so no reference outlives this main-thread call.
    if unsafe {
        anchor
            .data::<FocusMapHandlerInstalled>(FOCUS_MAP_HANDLER_INSTALLED_DATA_KEY)
            .is_none()
    } {
        // SAFETY: same key/type pairing as the check above; widget data is
        // only touched on the GTK main thread.
        unsafe {
            anchor.set_data(
                FOCUS_MAP_HANDLER_INSTALLED_DATA_KEY,
                FocusMapHandlerInstalled,
            );
        };
        anchor.connect_map(|widget| {
            // SAFETY: `FOCUS_REQUEST_PENDING_DATA_KEY` only ever stores
            // `PendingFocusRequest`; stealing transfers ownership of the
            // marker to this main-thread call, and no other reference to it
            // exists because presence checks discard their pointers.
            if unsafe { widget.steal_data::<PendingFocusRequest>(FOCUS_REQUEST_PENDING_DATA_KEY) }
                .is_none()
            {
                return;
            }

            assert!(
                widget.grab_focus(),
                "GTK Focused metadata failed to focus its resolved TextField/SecureField anchor when it was mapped"
            );
        });
    }
}

fn clear_focus(anchor: &Widget) {
    // SAFETY: `FOCUS_REQUEST_PENDING_DATA_KEY` only ever stores
    // `PendingFocusRequest`; stealing transfers ownership of the marker to
    // this main-thread call, and no other reference to it exists because
    // presence checks discard their pointers.
    let _ = unsafe { anchor.steal_data::<PendingFocusRequest>(FOCUS_REQUEST_PENDING_DATA_KEY) };

    if !anchor.has_focus() {
        return;
    }

    let Some(root) = anchor.root() else {
        panic!("focused anchor lost its GTK root while still holding focus");
    };
    root.set_focus(None::<&Widget>);
}

/// Context passed to component renderers.
///
/// Only [`GtkRenderer::render`] and [`GtkRenderer::render_any`] construct
/// this, always from a live `&mut GtkRenderer` immediately before a
/// synchronous dispatch, so the pointer is never null and outlives every
/// handler invocation it is passed to.
#[derive(Debug, Clone)]
pub struct RenderContext {
    /// Reference to the renderer for recursive rendering.
    /// This is a raw pointer because we can't have self-referential borrows.
    renderer_ptr: *mut GtkRenderer,
}

impl RenderContext {
    /// Creates a context with a renderer reference.
    const fn with_renderer(renderer: &mut GtkRenderer) -> Self {
        Self {
            renderer_ptr: std::ptr::from_mut::<GtkRenderer>(renderer),
        }
    }

    /// Gets a mutable reference to the renderer.
    ///
    /// # Safety
    ///
    /// The caller must ensure the renderer pointer is valid.
    #[allow(
        clippy::mut_from_ref,
        reason = "RenderContext is a raw-pointer handle threaded through GTK callbacks; \
                  the caller upholds exclusivity, which the signature cannot express"
    )]
    pub(crate) unsafe fn renderer(&self) -> &mut GtkRenderer {
        // SAFETY: Caller guarantees the renderer pointer is initialized and valid.
        unsafe { &mut *self.renderer_ptr }
    }
}

const GTK_METADATA_CSS_PRIORITY: u32 = gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION;
const CSS_CLASS_BORDER: &str = "waterui-metadata-border";
const CSS_CLASS_SHADOW: &str = "waterui-metadata-shadow";
const CSS_CLASS_SCALE: &str = "waterui-metadata-scale";
const CSS_CLASS_ROTATION: &str = "waterui-metadata-rotation";
const CSS_CLASS_OFFSET: &str = "waterui-metadata-offset";
const CSS_CLASS_CLIP_SHAPE: &str = "waterui-metadata-clip-shape";

#[derive(Debug, Clone, Copy)]
enum ClipShapeCss {
    Rectangle,
    Ellipse,
    Rounded {
        top_left: f32,
        top_right: f32,
        bottom_right: f32,
        bottom_left: f32,
    },
}

fn wrap_for_metadata(child: &Widget) -> gtk4::Box {
    let wrapper = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    wrapper.set_halign(Align::Fill);
    wrapper.set_valign(Align::Fill);
    wrapper.append(child);
    wrapper
}

fn attach_css_provider(widget: &impl IsA<gtk4::Widget>, class_name: &str) -> ScopedCss {
    ScopedCss::attach(widget, class_name, GTK_METADATA_CSS_PRIORITY)
}

fn cursor_style_to_gtk_name(style: CursorStyle) -> &'static str {
    match style {
        CursorStyle::Arrow => "default",
        CursorStyle::PointingHand => "pointer",
        CursorStyle::IBeam => "text",
        CursorStyle::Crosshair => "crosshair",
        CursorStyle::OpenHand => "grab",
        CursorStyle::ClosedHand => "grabbing",
        CursorStyle::NotAllowed => "not-allowed",
        CursorStyle::ResizeLeft => "w-resize",
        CursorStyle::ResizeRight => "e-resize",
        CursorStyle::ResizeUp => "n-resize",
        CursorStyle::ResizeDown => "s-resize",
        CursorStyle::ResizeLeftRight => "ew-resize",
        CursorStyle::ResizeUpDown => "ns-resize",
        CursorStyle::Move => "move",
        CursorStyle::Wait => "wait",
        CursorStyle::Copy => "copy",
        _ => panic!("unsupported CursorStyle variant on GTK backend"),
    }
}

fn apply_shadow_css(css: &ScopedCss, resolved: ResolvedColor, x: f32, y: f32, blur: f32) {
    let color = crate::util::resolved_color_to_css_rgba(resolved);
    css.set_declarations(&format!(
        "box-shadow: {x:.2}px {y:.2}px {blur:.2}px {color};"
    ));
}

fn apply_border_css(
    css: &ScopedCss,
    resolved: ResolvedColor,
    width: f32,
    corner_radius: f32,
    edges: waterui_layout::EdgeSet,
) {
    let color = crate::util::resolved_color_to_css_rgba(resolved);
    let top = if edges.top { width } else { 0.0 };
    let left = if edges.leading { width } else { 0.0 };
    let bottom = if edges.bottom { width } else { 0.0 };
    let right = if edges.trailing { width } else { 0.0 };
    css.set_declarations(&format!(
        "border-style: solid; border-color: {color}; \
         border-top-width: {top:.2}px; border-left-width: {left:.2}px; \
         border-bottom-width: {bottom:.2}px; border-right-width: {right:.2}px; \
         border-radius: {corner_radius:.2}px;"
    ));
}

fn apply_scale_css(css: &ScopedCss, x: f32, y: f32, anchor: waterui::style::Anchor) {
    let origin_x = anchor.x * 100.0;
    let origin_y = anchor.y * 100.0;
    css.set_declarations(&format!(
        "transform: scale({x:.5}, {y:.5}); transform-origin: {origin_x:.3}% {origin_y:.3}%;"
    ));
}

fn apply_rotation_css(css: &ScopedCss, angle_degrees: f32, anchor: waterui::style::Anchor) {
    let origin_x = anchor.x * 100.0;
    let origin_y = anchor.y * 100.0;
    css.set_declarations(&format!(
        "transform: rotate({angle_degrees:.5}deg); transform-origin: {origin_x:.3}% {origin_y:.3}%;"
    ));
}

fn apply_offset_css(css: &ScopedCss, x: f32, y: f32) {
    css.set_declarations(&format!("transform: translate({x:.2}px, {y:.2}px);"));
}

fn apply_clip_shape_css(css: &ScopedCss, shape: ClipShapeCss) {
    let body = match shape {
        ClipShapeCss::Rectangle => "overflow: hidden;",
        ClipShapeCss::Ellipse => "overflow: hidden; border-radius: 50%;",
        ClipShapeCss::Rounded {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } => {
            let tl = top_left * 100.0;
            let tr = top_right * 100.0;
            let br = bottom_right * 100.0;
            let bl = bottom_left * 100.0;
            return css.set_declarations(&format!(
                "overflow: hidden; border-radius: {tl:.3}% {tr:.3}% {br:.3}% {bl:.3}%;"
            ));
        }
    };
    css.set_declarations(body);
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3
}

#[allow(
    clippy::too_many_lines,
    reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
)]
fn clip_shape_css(shape: &ClipShape) -> ClipShapeCss {
    let commands = shape.commands();
    match commands {
        [
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 1.0 },
            PathCommand::LineTo { x: 0.0, y: 1.0 },
            PathCommand::Close,
        ] => ClipShapeCss::Rectangle,
        [
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            },
        ] if approx_eq(*cx, 0.5)
            && approx_eq(*cy, 0.5)
            && approx_eq(*rx, 0.5)
            && approx_eq(*ry, 0.5)
            && approx_eq(*start, 0.0)
            && approx_eq(*sweep, std::f32::consts::TAU) =>
        {
            ClipShapeCss::Ellipse
        }
        [
            PathCommand::MoveTo { x: tl, y: 0.0 },
            PathCommand::LineTo { x: x2, y: 0.0 },
            PathCommand::Arc {
                cx: _,
                cy: tr,
                rx: top_right_radius_x,
                ry: top_right_radius_y,
                start: _,
                sweep: _,
            },
            PathCommand::LineTo { x: 1.0, y: y4 },
            PathCommand::Arc {
                cx: _,
                cy: _,
                rx: bottom_right_radius_x,
                ry: bottom_right_radius_y,
                start: _,
                sweep: _,
            },
            PathCommand::LineTo { x: bl, y: 1.0 },
            PathCommand::Arc {
                cx: _,
                cy: _,
                rx: bottom_left_radius_x,
                ry: bottom_left_radius_y,
                start: _,
                sweep: _,
            },
            PathCommand::LineTo { x: 0.0, y: y8 },
            PathCommand::Arc {
                cx: _,
                cy: _,
                rx: top_left_radius_x,
                ry: top_left_radius_y,
                start: _,
                sweep: _,
            },
            PathCommand::Close,
        ] => {
            let top_left = (*tl + *top_left_radius_x + *top_left_radius_y + *y8) / 4.0;
            let top_right = ((1.0 - *x2) + *tr + *top_right_radius_x + *top_right_radius_y) / 4.0;
            let bottom_right =
                ((1.0 - *y4) + *bottom_right_radius_x + *bottom_right_radius_y) / 3.0;
            let bottom_left = (*bl + *bottom_left_radius_x + *bottom_left_radius_y) / 3.0;
            ClipShapeCss::Rounded {
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            }
        }
        [
            PathCommand::MoveTo { x: 0.5, y: 0.0 },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: _,
                sweep: _,
            },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: _,
                sweep: _,
            },
            PathCommand::Close,
        ] => ClipShapeCss::Ellipse,
        _ => panic!("unsupported ClipShape path for GTK backend"),
    }
}

fn drag_content_provider(data: &DragData) -> gdk4::ContentProvider {
    let text = data.as_str().to_owned();
    let text_provider = gdk4::ContentProvider::for_value(&text.to_value());
    match data {
        DragData::Text(_) => text_provider,
        DragData::Url(_) => {
            let uri_list = format!("{text}\r\n");
            let uri_provider = gdk4::ContentProvider::for_bytes(
                "text/uri-list",
                &glib::Bytes::from(uri_list.as_bytes()),
            );
            gdk4::ContentProvider::new_union(&[text_provider, uri_provider])
        }
        _ => panic!("unsupported DragData variant on GTK backend"),
    }
}

fn drag_data_from_text(text: String) -> DragData {
    if text.starts_with("http://") || text.starts_with("https://") || text.starts_with("file://") {
        DragData::url(text)
    } else {
        DragData::text(text)
    }
}

fn drag_data_from_drop_value(value: &glib::Value) -> Option<DragData> {
    if let Ok(text) = value.get::<String>() {
        return Some(drag_data_from_text(text));
    }
    if let Ok(text) = value.get::<glib::GString>() {
        return Some(drag_data_from_text(text.to_string()));
    }
    None
}

fn call_boxed_action(action: &Rc<RefCell<BoxedAction<()>>>, env: &Environment) {
    if let Ok(mut handler) = action.try_borrow_mut() {
        (**handler)(env);
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
)]
#[allow(
    clippy::too_many_lines,
    reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
)]
fn install_gesture_observer(
    widget: &Widget,
    gesture: waterui::gesture::Gesture,
    action: Rc<RefCell<BoxedAction<()>>>,
    env: Environment,
) {
    use waterui::gesture::{
        DragEvent, Gesture, GesturePhase, GesturePoint, LongPressEvent, MagnificationEvent,
        RotationEvent, TapEvent,
    };

    match gesture {
        Gesture::Tap(tap) => {
            let click = gtk4::GestureClick::new();
            click.set_button(1);
            click.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let required_count = tap.count;
            let env_for_handler = env;
            let action_for_handler = action.clone();
            click.connect_pressed(move |gesture, n_press, x, y| {
                if n_press as u32 >= required_count {
                    let tap_event = TapEvent {
                        location: GesturePoint::new(x as f32, y as f32),
                        count: n_press as u32,
                    };
                    let mut local_env = env_for_handler.clone();
                    local_env.insert(tap_event);
                    call_boxed_action(&action_for_handler, &local_env);
                    gesture.set_state(gtk4::EventSequenceState::Claimed);
                }
            });
            widget.add_controller(click);
        }
        Gesture::LongPress(long_press) => {
            let press = gtk4::GestureLongPress::new();
            press.set_delay_factor(f64::from(long_press.duration) / 500.0);
            press.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let env_for_handler = env;
            let action_for_handler = action.clone();
            let duration = long_press.duration;
            press.connect_pressed(move |gesture, x, y| {
                let event = LongPressEvent {
                    location: GesturePoint::new(x as f32, y as f32),
                    duration: duration as f32,
                };
                let mut local_env = env_for_handler.clone();
                local_env.insert(event);
                call_boxed_action(&action_for_handler, &local_env);
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            });
            widget.add_controller(press);
        }
        Gesture::Drag(drag) => {
            let drag_gesture = gtk4::GestureDrag::new();
            drag_gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
            let min_distance = drag.min_distance;
            let drag_started = Rc::new(RefCell::new(false));
            {
                let env_for_handler = env.clone();
                let action_for_handler = action.clone();
                let drag_started = drag_started.clone();
                drag_gesture.connect_drag_begin(move |gesture, x, y| {
                    *drag_started.borrow_mut() = false;
                    let event = DragEvent {
                        phase: GesturePhase::Started,
                        location: GesturePoint::new(x as f32, y as f32),
                        translation: GesturePoint::new(0.0, 0.0),
                        velocity: GesturePoint::new(0.0, 0.0),
                    };
                    let mut local_env = env_for_handler.clone();
                    local_env.insert(event);
                    call_boxed_action(&action_for_handler, &local_env);
                    gesture.set_state(gtk4::EventSequenceState::Claimed);
                });
            }
            {
                let env_for_handler = env.clone();
                let action_for_handler = action.clone();
                let drag_started = drag_started.clone();
                drag_gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
                    let distance = offset_x.hypot(offset_y) as f32;
                    if distance < min_distance && !*drag_started.borrow() {
                        return;
                    }
                    *drag_started.borrow_mut() = true;
                    let start = gesture.start_point().unwrap_or((0.0, 0.0));
                    let event = DragEvent {
                        phase: GesturePhase::Updated,
                        location: GesturePoint::new(
                            (start.0 + offset_x) as f32,
                            (start.1 + offset_y) as f32,
                        ),
                        translation: GesturePoint::new(offset_x as f32, offset_y as f32),
                        velocity: GesturePoint::new(0.0, 0.0),
                    };
                    let mut local_env = env_for_handler.clone();
                    local_env.insert(event);
                    call_boxed_action(&action_for_handler, &local_env);
                });
            }
            {
                let env_for_handler = env;
                let action_for_handler = action.clone();
                drag_gesture.connect_drag_end(move |gesture, offset_x, offset_y| {
                    if !*drag_started.borrow() {
                        return;
                    }
                    let start = gesture.start_point().unwrap_or((0.0, 0.0));
                    let event = DragEvent {
                        phase: GesturePhase::Ended,
                        location: GesturePoint::new(
                            (start.0 + offset_x) as f32,
                            (start.1 + offset_y) as f32,
                        ),
                        translation: GesturePoint::new(offset_x as f32, offset_y as f32),
                        velocity: GesturePoint::new(0.0, 0.0),
                    };
                    let mut local_env = env_for_handler.clone();
                    local_env.insert(event);
                    call_boxed_action(&action_for_handler, &local_env);
                });
            }
            widget.add_controller(drag_gesture);
        }
        Gesture::Magnification(_) => {
            let zoom = gtk4::GestureZoom::new();
            let env_for_handler = env;
            let action_for_handler = action.clone();
            zoom.connect_scale_changed(move |gesture, scale| {
                let center = gesture.bounding_box().map_or_else(
                    || GesturePoint::new(0.0, 0.0),
                    |bbox| GesturePoint::new(bbox.x() as f32, bbox.y() as f32),
                );
                let event = MagnificationEvent {
                    phase: GesturePhase::Updated,
                    center,
                    scale: scale as f32,
                    velocity: 0.0,
                };
                let mut local_env = env_for_handler.clone();
                local_env.insert(event);
                call_boxed_action(&action_for_handler, &local_env);
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            });
            widget.add_controller(zoom);
        }
        Gesture::Rotation(_) => {
            let rotate = gtk4::GestureRotate::new();
            let env_for_handler = env;
            let action_for_handler = action.clone();
            rotate.connect_angle_changed(move |gesture, angle, _delta| {
                let center = gesture.bounding_box().map_or_else(
                    || GesturePoint::new(0.0, 0.0),
                    |bbox| GesturePoint::new(bbox.x() as f32, bbox.y() as f32),
                );
                let event = RotationEvent {
                    phase: GesturePhase::Updated,
                    center,
                    angle: angle as f32,
                    // GTK reports no angular velocity, matching the zoom gesture.
                    velocity: 0.0,
                };
                let mut local_env = env_for_handler.clone();
                local_env.insert(event);
                call_boxed_action(&action_for_handler, &local_env);
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            });
            widget.add_controller(rotate);
        }
        Gesture::Then(then) => {
            let armed = Rc::new(RefCell::new(false));
            let first_action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new({
                let armed = armed.clone();
                move |_env: &Environment| {
                    *armed.borrow_mut() = true;
                }
            })));
            let second_action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new({
                let chained_action = action.clone();
                move |env: &Environment| {
                    if !*armed.borrow() {
                        return;
                    }
                    *armed.borrow_mut() = false;
                    call_boxed_action(&chained_action, env);
                }
            })));
            install_gesture_observer(widget, then.first().clone(), first_action, env.clone());
            install_gesture_observer(widget, then.then().clone(), second_action, env);
        }
        Gesture::Simultaneous(simultaneous) => {
            install_gesture_observer(
                widget,
                simultaneous.first().clone(),
                action.clone(),
                env.clone(),
            );
            install_gesture_observer(widget, simultaneous.second().clone(), action, env);
        }
        Gesture::Exclusive(exclusive) => {
            let consumed = Rc::new(RefCell::new(false));
            let first_action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new({
                let consumed = consumed.clone();
                let shared_action = action.clone();
                move |env: &Environment| {
                    *consumed.borrow_mut() = true;
                    call_boxed_action(&shared_action, env);
                    let consumed = consumed.clone();
                    glib::idle_add_local_once(move || {
                        *consumed.borrow_mut() = false;
                    });
                }
            })));
            let second_action: Rc<RefCell<BoxedAction<()>>> = Rc::new(RefCell::new(Box::new({
                let shared_action = action.clone();
                move |env: &Environment| {
                    if *consumed.borrow() {
                        return;
                    }
                    call_boxed_action(&shared_action, env);
                    let consumed = consumed.clone();
                    glib::idle_add_local_once(move || {
                        *consumed.borrow_mut() = false;
                    });
                }
            })));
            install_gesture_observer(widget, exclusive.first().clone(), first_action, env.clone());
            install_gesture_observer(widget, exclusive.second().clone(), second_action, env);
        }
        _ => panic!("unsupported Gesture variant on GTK backend"),
    }
}

/// GTK renderer that converts `WaterUI` views to GTK widgets.
#[derive(Debug)]
pub struct GtkRenderer {
    dispatcher: ViewDispatcher<(), RenderContext, Widget>,
}

impl GtkRenderer {
    /// Creates a new GTK renderer with all component handlers registered.
    #[must_use]
    pub fn new() -> Self {
        let mut dispatcher = ViewDispatcher::new();

        // Register component handlers
        Self::register_components(&mut dispatcher);

        Self { dispatcher }
    }

    /// Renders a view to a GTK widget.
    pub fn render<V: View>(&mut self, view: V, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch(view, env, ctx)
    }

    /// Renders an `AnyView` to a GTK widget.
    pub fn render_any(&mut self, view: AnyView, env: &Environment) -> Widget {
        let ctx = RenderContext::with_renderer(self);
        self.dispatcher.dispatch(view, env, ctx)
    }

    fn register_components(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        // Register Native<T> wrapped components
        Self::register_native::<TextConfig>(dispatcher);
        Self::register_native::<Spacer>(dispatcher);
        Self::register_native::<FixedContainer>(dispatcher);
        Self::register_native::<LazyContainer>(dispatcher);
        Self::register_native::<ButtonConfig>(dispatcher);
        Self::register_native::<ToggleConfig>(dispatcher);
        Self::register_native::<SliderConfig>(dispatcher);
        Self::register_native::<ResolvedTextFieldConfig>(dispatcher);
        Self::register_native::<ProgressConfig>(dispatcher);
        Self::register_native::<StepperConfig>(dispatcher);
        Self::register_native::<ScrollView>(dispatcher);
        Self::register_native::<Tabs>(dispatcher);
        Self::register_native::<ListConfig>(dispatcher);
        Self::register_native::<SecureFieldConfig>(dispatcher);
        Self::register_native::<PickerConfig>(dispatcher);
        Self::register_native::<DatePickerConfig>(dispatcher);
        Self::register_native::<MultiDatePickerConfig>(dispatcher);
        Self::register_native::<ColorPickerConfig>(dispatcher);
        Self::register_native::<ResolvedMenu>(dispatcher);
        Self::register_native::<SystemIcon>(dispatcher);
        #[cfg(feature = "chromium")]
        Self::register::<ChromiumView>(dispatcher);
        #[cfg(any(
            feature = "webview-default",
            feature = "webview-system",
            feature = "webview-wpe",
            feature = "webview-cef"
        ))]
        Self::register_native::<WebView>(dispatcher);
        Self::register_native::<Color>(dispatcher);
        Self::register_native::<ResolvedColor>(dispatcher);
        Self::register_native::<ResolvedGradient>(dispatcher);
        Self::register_native::<ResolvedShape>(dispatcher);

        // Register Dynamic for reactive content
        Self::register::<Native<Dynamic>>(dispatcher);

        // Register GPU surface (used by waterui-graphics and waterui-media)
        Self::register_native::<GpuSurface>(dispatcher);

        // Register views that implement View directly
        Self::register::<Divider>(dispatcher);
        Self::register::<NavigationView>(dispatcher);
        Self::register::<NavigationStack<(), ()>>(dispatcher);
        Self::register::<NavigationSplitLayout>(dispatcher);

        // Register metadata handlers
        Self::register_metadata_handlers(dispatcher);

        // Register Str directly (before it wraps into Native<Str>)
        Self::register_str_handler(dispatcher);

        // Register unit type () as empty widget
        Self::register_unit_handler(dispatcher);
    }

    /// Registers a handler for `Str` that renders it as a GTK Label.
    fn register_str_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Str>(|_state, _ctx, s, _env| {
            let label = gtk4::Label::new(Some(s.as_str()));
            // Let text maintain natural width - layout system handles sizing
            label.upcast()
        });
    }

    /// Registers a handler for unit type `()` as an empty widget.
    fn register_unit_handler(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        dispatcher.register::<Native<()>>(|_state, _ctx, _unit, _env| {
            // Return an empty widget (invisible box with no children)
            let empty = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            empty.set_visible(false);
            empty.upcast()
        });
    }

    /// Registers handlers for metadata wrapper views.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "one cohesive widget-construction pass; splitting it would scatter GTK setup order"
    )]
    fn register_metadata_handlers(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        use waterui::component::focus::Focused;
        use waterui::filter::Opacity;
        use waterui::gesture::GestureObserver;
        use waterui::style::Shadow;

        // Metadata<Environment> - use provided environment for subtree
        Self::register_with_renderer::<Metadata<Environment>>(
            dispatcher,
            |renderer, metadata, _env| renderer.render_any(metadata.content, &metadata.value),
        );

        // Metadata<Retain> - keep retained value alive for the widget lifetime
        Self::register_with_renderer::<Metadata<Retain>>(dispatcher, |renderer, metadata, env| {
            let widget = renderer.render_any(metadata.content, env);
            // SAFETY: `RETAIN_DATA_KEY` is written here only and never read
            // back; the value is stored purely so the widget's destruction
            // drops it, and widget data lives on the GTK main thread.
            unsafe { widget.set_data(RETAIN_DATA_KEY, metadata.value) };
            widget
        });

        // Metadata<LifeCycleHook> - invoke hook on appear/disappear
        Self::register_with_renderer::<Metadata<LifeCycleHook>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                match metadata.value.lifecycle() {
                    LifeCycle::Appear => {
                        let mut hook = Some(metadata.value);
                        let env = env.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(hook) = hook.take() {
                                hook.handle(&env);
                            }
                        });
                    }
                    LifeCycle::Disappear => {
                        let hook = Rc::new(RefCell::new(Some(metadata.value)));
                        let env = env.clone();
                        widget.connect_unrealize(move |_| {
                            if let Some(hook) = hook.borrow_mut().take() {
                                hook.handle(&env);
                            }
                        });
                    }
                    _ => panic!("unsupported LifeCycle variant on GTK backend"),
                }
                widget
            },
        );

        // Metadata<Opacity> - apply opacity via GTK widget opacity
        Self::register_with_renderer::<Metadata<Opacity>>(dispatcher, |renderer, metadata, env| {
            let widget = renderer.render_any(metadata.content, env);
            let alpha = metadata.value.value;
            let (initial, guard) = subscribe_then_get(&alpha, {
                let widget = widget.clone();
                move |ctx| {
                    let alpha = ctx.into_value();
                    let widget = widget.clone();
                    glib::idle_add_local_once(move || {
                        widget.set_opacity(f64::from(alpha));
                    });
                }
            });
            widget.set_opacity(f64::from(initial));
            store_watcher_guard(&widget, Box::new(guard));
            widget
        });

        // Metadata<Shadow> - apply CSS shadow to a wrapper
        Self::register_with_renderer::<Metadata<Shadow>>(dispatcher, |renderer, metadata, env| {
            let content = renderer.render_any(metadata.content, env);
            let wrapper = wrap_for_metadata(&content);
            let scoped_css = attach_css_provider(&wrapper, CSS_CLASS_SHADOW);
            let shadow = metadata.value;
            let color_signal = shadow.color.resolve(env);
            let (initial, guard) = subscribe_then_get(&color_signal, {
                let scoped_css = scoped_css.clone();
                move |ctx| {
                    let resolved = ctx.into_value();
                    let scoped_css = scoped_css.clone();
                    glib::idle_add_local_once(move || {
                        apply_shadow_css(
                            &scoped_css,
                            resolved,
                            shadow.offset.x,
                            shadow.offset.y,
                            shadow.radius,
                        );
                    });
                }
            });
            apply_shadow_css(
                &scoped_css,
                initial,
                shadow.offset.x,
                shadow.offset.y,
                shadow.radius,
            );
            store_watcher_guard(&wrapper, Box::new(guard));
            wrapper.upcast()
        });

        // Metadata<Focused> - bridge focus state with GTK focus
        Self::register_with_renderer::<Metadata<Focused>>(dispatcher, |renderer, metadata, env| {
            let widget = renderer.render_any(metadata.content, env);
            attach_focus_metadata(widget, &metadata.value.0)
        });

        // Metadata<Cursor> - update pointer cursor while hovering
        Self::register_with_renderer::<Metadata<Cursor>>(dispatcher, |renderer, metadata, env| {
            let widget = renderer.render_any(metadata.content, env);
            widget.set_can_target(true);
            let style_signal = metadata.value.style;
            let hovered = Rc::new(Cell::new(false));
            let style_state = Rc::new(Cell::new(None));
            let (initial_style, guard) = subscribe_then_get(&style_signal, {
                let style_state = style_state.clone();
                let hovered = hovered.clone();
                let widget = widget.clone();
                move |ctx| {
                    let style = ctx.into_value();
                    style_state.set(Some(style));
                    if hovered.get() {
                        let widget = widget.clone();
                        glib::idle_add_local_once(move || {
                            widget.set_cursor_from_name(Some(cursor_style_to_gtk_name(style)));
                        });
                    }
                }
            });
            style_state.set(Some(initial_style));
            let motion = gtk4::EventControllerMotion::new();
            {
                let hovered = hovered.clone();
                let widget = widget.clone();
                motion.connect_enter(move |_, _, _| {
                    hovered.set(true);
                    let style = style_state
                        .get()
                        .expect("GTK cursor signal must be initialized before pointer entry");
                    widget.set_cursor_from_name(Some(cursor_style_to_gtk_name(style)));
                });
            }
            {
                let widget = widget.clone();
                motion.connect_leave(move |_| {
                    hovered.set(false);
                    widget.set_cursor_from_name(None);
                });
            }
            widget.add_controller(motion);
            store_watcher_guard(&widget, Box::new(guard));
            widget
        });

        // Metadata<Border> - apply CSS border to a wrapper
        Self::register_with_renderer::<Metadata<Border>>(dispatcher, |renderer, metadata, env| {
            let content = renderer.render_any(metadata.content, env);
            let wrapper = wrap_for_metadata(&content);
            let scoped_css = attach_css_provider(&wrapper, CSS_CLASS_BORDER);
            let border = metadata.value;
            let color_signal = border.color.resolve(env);
            let (initial, guard) = subscribe_then_get(&color_signal, {
                let scoped_css = scoped_css.clone();
                move |ctx| {
                    let resolved = ctx.into_value();
                    let scoped_css = scoped_css.clone();
                    glib::idle_add_local_once(move || {
                        apply_border_css(
                            &scoped_css,
                            resolved,
                            border.width,
                            border.corner_radius,
                            border.edges,
                        );
                    });
                }
            });
            apply_border_css(
                &scoped_css,
                initial,
                border.width,
                border.corner_radius,
                border.edges,
            );
            store_watcher_guard(&wrapper, Box::new(guard));
            wrapper.upcast()
        });

        // Metadata<Scale> - visual scale transform wrapper
        Self::register_with_renderer::<Metadata<Scale>>(dispatcher, |renderer, metadata, env| {
            let content = renderer.render_any(metadata.content, env);
            let wrapper = wrap_for_metadata(&content);
            let scoped_css = attach_css_provider(&wrapper, CSS_CLASS_SCALE);
            let scale = metadata.value;
            let values = Rc::new(ReactiveAxisPair::default());
            let (initial_x, x_guard) = subscribe_then_get(&scale.x, {
                let scoped_css = scoped_css.clone();
                let values = values.clone();
                move |ctx| {
                    values.update_x(ctx.into_value());
                    if let Some((x, y)) = values.values() {
                        let scoped_css = scoped_css.clone();
                        glib::idle_add_local_once(move || {
                            apply_scale_css(&scoped_css, x, y, scale.anchor);
                        });
                    }
                }
            });
            values.update_x(initial_x);
            let (initial_y, y_guard) = subscribe_then_get(&scale.y, {
                let scoped_css = scoped_css.clone();
                let values = values.clone();
                move |ctx| {
                    values.update_y(ctx.into_value());
                    if let Some((x, y)) = values.values() {
                        let scoped_css = scoped_css.clone();
                        glib::idle_add_local_once(move || {
                            apply_scale_css(&scoped_css, x, y, scale.anchor);
                        });
                    }
                }
            });
            values.update_y(initial_y);
            let (x, y) = values
                .values()
                .expect("GTK scale signals must be initialized after subscription");
            apply_scale_css(&scoped_css, x, y, scale.anchor);
            crate::util::store_watcher_guards(&wrapper, vec![Box::new(x_guard), Box::new(y_guard)]);
            wrapper.upcast()
        });

        // Metadata<Rotation> - visual rotation transform wrapper
        Self::register_with_renderer::<Metadata<Rotation>>(
            dispatcher,
            |renderer, metadata, env| {
                let content = renderer.render_any(metadata.content, env);
                let wrapper = wrap_for_metadata(&content);
                let scoped_css = attach_css_provider(&wrapper, CSS_CLASS_ROTATION);
                let rotation = metadata.value;
                let (initial, guard) = subscribe_then_get(&rotation.angle, {
                    let scoped_css = scoped_css.clone();
                    move |ctx| {
                        let angle = ctx.into_value();
                        let scoped_css = scoped_css.clone();
                        glib::idle_add_local_once(move || {
                            apply_rotation_css(&scoped_css, angle, rotation.anchor);
                        });
                    }
                });
                apply_rotation_css(&scoped_css, initial, rotation.anchor);
                store_watcher_guard(&wrapper, Box::new(guard));
                wrapper.upcast()
            },
        );

        // Metadata<Offset> - visual translate transform wrapper
        Self::register_with_renderer::<Metadata<Offset>>(dispatcher, |renderer, metadata, env| {
            let content = renderer.render_any(metadata.content, env);
            let wrapper = wrap_for_metadata(&content);
            let scoped_css = attach_css_provider(&wrapper, CSS_CLASS_OFFSET);
            let offset = metadata.value;
            let values = Rc::new(ReactiveAxisPair::default());
            let (initial_x, x_guard) = subscribe_then_get(&offset.x, {
                let scoped_css = scoped_css.clone();
                let values = values.clone();
                move |ctx| {
                    values.update_x(ctx.into_value());
                    if let Some((x, y)) = values.values() {
                        let scoped_css = scoped_css.clone();
                        glib::idle_add_local_once(move || {
                            apply_offset_css(&scoped_css, x, y);
                        });
                    }
                }
            });
            values.update_x(initial_x);
            let (initial_y, y_guard) = subscribe_then_get(&offset.y, {
                let scoped_css = scoped_css.clone();
                let values = values.clone();
                move |ctx| {
                    values.update_y(ctx.into_value());
                    if let Some((x, y)) = values.values() {
                        let scoped_css = scoped_css.clone();
                        glib::idle_add_local_once(move || {
                            apply_offset_css(&scoped_css, x, y);
                        });
                    }
                }
            });
            values.update_y(initial_y);
            let (x, y) = values
                .values()
                .expect("GTK offset signals must be initialized after subscription");
            apply_offset_css(&scoped_css, x, y);
            crate::util::store_watcher_guards(&wrapper, vec![Box::new(x_guard), Box::new(y_guard)]);
            wrapper.upcast()
        });

        // Metadata<ClipShape> - clip content for known canonical shapes
        Self::register_with_renderer::<Metadata<ClipShape>>(
            dispatcher,
            |renderer, metadata, env| {
                let content = renderer.render_any(metadata.content, env);
                let wrapper = wrap_for_metadata(&content);
                wrapper.set_overflow(Overflow::Hidden);
                let scoped_css = attach_css_provider(&wrapper, CSS_CLASS_CLIP_SHAPE);
                let css = clip_shape_css(&metadata.value);
                apply_clip_shape_css(&scoped_css, css);
                wrapper.upcast()
            },
        );

        // Metadata<Secure> - passthrough (GTK cannot enforce screenshot protection)
        Self::register_passthrough_metadata::<Secure>(dispatcher);

        // Metadata<StandardDynamicRange> / Metadata<HighDynamicRange> - passthrough on GTK
        Self::register_passthrough_metadata::<StandardDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<HighDynamicRange>(dispatcher);

        // Metadata<OnEvent> - handle hover events
        Self::register_with_renderer::<Metadata<OnEvent>>(dispatcher, |renderer, metadata, env| {
            let widget = renderer.render_any(metadata.content, env);
            widget.set_can_target(true);
            let expected = metadata.value.event();
            let handler = Rc::new(RefCell::new(metadata.value));
            let env = env.clone();
            let motion = gtk4::EventControllerMotion::new();
            match expected {
                Event::HoverEnter => {
                    let env = env;
                    let handler = handler;
                    motion.connect_enter(move |_, _, _| {
                        if let Ok(mut on_event) = handler.try_borrow_mut() {
                            on_event.handle(&env);
                        }
                    });
                }
                Event::HoverMove => {
                    let env = env;
                    let handler = handler;
                    motion.connect_motion(move |_, x, y| {
                        if let Ok(mut on_event) = handler.try_borrow_mut() {
                            let hover_env = env.extending(HoverEvent::new(
                                waterui_core::layout::Point::new(x as f32, y as f32),
                            ));
                            on_event.handle(&hover_env);
                        }
                    });
                }
                Event::HoverExit => {
                    let env = env;
                    let handler = handler;
                    motion.connect_leave(move |_| {
                        if let Ok(mut on_event) = handler.try_borrow_mut() {
                            on_event.handle(&env);
                        }
                    });
                }
                _ => panic!("unsupported OnEvent variant on GTK backend"),
            }
            widget.add_controller(motion);
            widget
        });

        // Metadata<GestureObserver> - attach gesture recognizers
        Self::register_with_renderer::<Metadata<GestureObserver>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                widget.set_can_target(true);
                let action = Rc::new(RefCell::new(metadata.value.action));
                install_gesture_observer(&widget, metadata.value.gesture, action, env.clone());
                widget
            },
        );

        // Metadata<ResolvedContextMenu> - right-click popover menu
        Self::register_with_renderer::<Metadata<ResolvedContextMenu>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                widget.set_can_target(true);
                let items = metadata.value.items;
                let popover_state: Rc<RefCell<Option<gtk4::Popover>>> = Rc::new(RefCell::new(None));
                let click = gtk4::GestureClick::new();
                click.set_button(3);
                click.connect_pressed({
                    let widget = widget.clone();
                    let env = env.clone();
                    let popover_state = popover_state;
                    move |_, _, x, y| {
                        let entries = items.get();
                        if entries.is_empty() {
                            return;
                        }
                        if let Some(existing) = popover_state.borrow_mut().take() {
                            existing.popdown();
                        }
                        let popover = gtk4::Popover::new();
                        popover.set_has_arrow(true);
                        popover.set_parent(&widget);
                        popover
                            .set_pointing_to(Some(&gdk4::Rectangle::new(x as i32, y as i32, 1, 1)));
                        rebuild_menu_popover(&popover, entries, &env);
                        popover.popup();
                        *popover_state.borrow_mut() = Some(popover);
                    }
                });
                widget.add_controller(click);
                widget
            },
        );

        // Metadata<Draggable> - native GTK drag source
        Self::register_with_renderer::<Metadata<Draggable>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                widget.set_can_target(true);
                let data = metadata.value.data;
                let source = gtk4::DragSource::new();
                source.set_actions(gdk4::DragAction::COPY);
                source.connect_prepare(move |_, _, _| {
                    let payload = data.get();
                    Some(drag_content_provider(&payload))
                });
                widget.add_controller(source);
                widget
            },
        );

        // Metadata<DropDestination> - native GTK drop target
        Self::register_with_renderer::<Metadata<DropDestination>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                widget.set_can_target(true);
                let drop = Rc::new(RefCell::new(metadata.value.on_drop));
                let enter = metadata
                    .value
                    .on_enter
                    .map(|handler| Rc::new(RefCell::new(handler)));
                let exit = metadata
                    .value
                    .on_exit
                    .map(|handler| Rc::new(RefCell::new(handler)));
                let target = gtk4::DropTarget::new(
                    String::static_type(),
                    gdk4::DragAction::COPY | gdk4::DragAction::MOVE,
                );
                target.set_types(&[String::static_type(), glib::GString::static_type()]);
                target.connect_enter({
                    let env = env.clone();
                    let enter = enter.clone();
                    move |_, _, _| {
                        if let Some(handler) = &enter
                            && let Ok(mut handler) = handler.try_borrow_mut()
                        {
                            (**handler)(&env);
                        }
                        gdk4::DragAction::COPY
                    }
                });
                target.connect_leave({
                    let env = env.clone();
                    let exit = exit.clone();
                    move |_| {
                        if let Some(handler) = &exit
                            && let Ok(mut handler) = handler.try_borrow_mut()
                        {
                            (**handler)(&env);
                        }
                    }
                });
                target.connect_drop({
                    let env = env.clone();
                    let drop = drop.clone();
                    move |_, value, _, _| {
                        let Some(data) = drag_data_from_drop_value(value) else {
                            return false;
                        };
                        let mut local_env = env.clone();
                        local_env.insert(data);
                        if let Ok(mut handler) = drop.try_borrow_mut() {
                            (**handler)(&local_env);
                            return true;
                        }
                        false
                    }
                });
                widget.add_controller(target);
                widget
            },
        );

        // Metadata<Hittable> - control hit testing and interaction
        Self::register_with_renderer::<Metadata<Hittable>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                let enabled = metadata.value.enabled;
                let (initial, guard) = subscribe_then_get(&enabled, {
                    let widget = widget.clone();
                    move |ctx| {
                        let enabled = ctx.into_value();
                        let widget = widget.clone();
                        glib::idle_add_local_once(move || {
                            widget.set_can_target(enabled);
                            widget.set_sensitive(enabled);
                        });
                    }
                });
                widget.set_can_target(initial);
                widget.set_sensitive(initial);
                store_watcher_guard(&widget, Box::new(guard));
                widget
            },
        );

        // Metadata<IgnoreSafeArea> - passthrough on GTK windowing model
        Self::register_passthrough_metadata::<IgnoreSafeArea>(dispatcher);

        // Metadata<Background> - passthrough for compatibility with metadata-based callers
        Self::register_passthrough_metadata::<Background>(dispatcher);

        Self::register_passthrough_metadata::<NavigationTransitionSource>(dispatcher);
        Self::register_passthrough_metadata::<NavigationTransitionDestination>(dispatcher);

        dispatcher.register::<Metadata<AppliedFilter>>(|_state, _ctx, _metadata, _env| {
            panic!(
                "AppliedFilter is unsupported on the GTK native backend because GTK does not \
                 expose its rendered widget texture to wgpu without a GPU-to-CPU readback; \
                 use a self-drawn backend for GPU filters"
            )
        });

        // Ignorable metadata with no native semantic realization.
        Self::register_ignorable_metadata::<MaterialBackground>(dispatcher);

        Self::register_with_renderer::<IgnorableMetadata<AccessibilityLabel>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                let weak = widget.downgrade();
                let (initial, guard) = subscribe_then_get(metadata.value.signal(), move |ctx| {
                    if let Some(widget) = weak.upgrade() {
                        let label = ctx.into_value();
                        widget
                            .update_property(&[gtk4::accessible::Property::Label(label.as_str())]);
                    }
                });
                widget.update_property(&[gtk4::accessible::Property::Label(initial.as_str())]);
                store_watcher_guard(&widget, Box::new(guard));
                widget
            },
        );
        Self::register_with_renderer::<IgnorableMetadata<AccessibilityRole>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                widget.set_accessible_role(gtk_accessibility_role(metadata.value));
                widget
            },
        );
        Self::register_with_renderer::<IgnorableMetadata<AccessibilityHidden>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                widget.update_state(&[gtk4::accessible::State::Hidden(metadata.value.is_hidden())]);
                widget
            },
        );
        Self::register_with_renderer::<IgnorableMetadata<AccessibilityChildren>>(
            dispatcher,
            |renderer, metadata, env| {
                let child = renderer.render_any(metadata.content, env);
                if !metadata.value.excludes_descendants() {
                    return child;
                }
                child.set_accessible_role(gtk4::AccessibleRole::Group);
                let mut descendant = child.first_child();
                while let Some(widget) = descendant {
                    widget.update_state(&[gtk4::accessible::State::Hidden(true)]);
                    descendant = widget.next_sibling();
                }
                child
            },
        );
        Self::register_with_renderer::<IgnorableMetadata<AccessibilityState>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                apply_gtk_accessibility_state(&widget, &metadata.value);
                widget
            },
        );
        Self::register_with_renderer::<IgnorableMetadata<AccessibilityStateSignal>>(
            dispatcher,
            |renderer, metadata, env| {
                let widget = renderer.render_any(metadata.content, env);
                let weak = widget.downgrade();
                let (initial, guard) = subscribe_then_get(metadata.value.state(), move |ctx| {
                    if let Some(widget) = weak.upgrade() {
                        apply_gtk_accessibility_state(&widget, &ctx.into_value());
                    }
                });
                apply_gtk_accessibility_state(&widget, &initial);
                store_watcher_guard(&widget, Box::new(guard));
                widget
            },
        );
    }

    /// Registers a handler for `Metadata<T>` that renders the content
    /// unchanged because the metadata has no GTK realization.
    fn register_passthrough_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
    ) where
        Metadata<T>: View,
    {
        Self::register_with_renderer::<Metadata<T>>(dispatcher, |renderer, metadata, env| {
            renderer.render_any(metadata.content, env)
        });
    }

    /// Registers a handler that receives the dispatching [`GtkRenderer`]
    /// directly, so handlers can recurse without touching the raw context
    /// pointer themselves.
    fn register_with_renderer<V: View>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
        handler: impl 'static + Clone + Fn(&mut Self, V, &Environment) -> Widget,
    ) {
        dispatcher.register::<V>(move |_state, ctx, view, env| {
            // SAFETY: every `RenderContext` is created by
            // `GtkRenderer::render`/`render_any` from a live `&mut GtkRenderer`
            // immediately before the synchronous `dispatch` call that invokes
            // this handler, and rendering runs exclusively on the GTK main
            // thread, so the pointer is valid for the whole handler invocation
            // and this is the only renderer reference used during it.
            let renderer = unsafe { ctx.renderer() };
            handler(renderer, view, env)
        });
    }

    /// Registers a handler for `IgnorableMetadata<T>` that just renders the content.
    fn register_ignorable_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
    ) {
        Self::register_with_renderer::<IgnorableMetadata<T>>(
            dispatcher,
            |renderer, metadata, env| renderer.render_any(metadata.content, env),
        );
    }

    /// Registers a `Native<T>` wrapped component with the dispatcher.
    fn register_native<T: waterui_core::NativeView + 'static>(
        dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>,
    ) where
        Native<T>: GtkComponent,
    {
        Self::register::<Native<T>>(dispatcher);
    }

    /// Registers a `GtkComponent` view type with the dispatcher.
    fn register<V: GtkComponent>(dispatcher: &mut ViewDispatcher<(), RenderContext, Widget>) {
        Self::register_with_renderer::<V>(dispatcher, |renderer, view, env| {
            view.render(env, renderer)
        });
    }
}

impl Default for GtkRenderer {
    fn default() -> Self {
        Self::new()
    }
}
