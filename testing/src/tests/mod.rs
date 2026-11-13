mod canvas;
mod harness;
mod settle;
mod shape_morph;
mod transitions;

use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use crate::driver::{DriverPumpResult, ResourceSampler};
use accesskit::{
    ActionRequest as AccessibilityActionRequest, Node as AccessibilityNode,
    NodeId as AccessibilityNodeId, Role as AccessibilityRole, TreeId as AccessibilityTreeId,
    TreeInfo as AccessibilityTreeInfo, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::InputEvent;
use waterui::Binding;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityGeometry;
use waterui::component::list::{List, ListItem};
use waterui::component::{text, vstack};
use waterui::graphics::WorkingColor;
use waterui::graphics::backend::draw::{self, Draw as _, Recorder};
use waterui::graphics::backend::{
    NextFrame, RecordingResources, SceneContent, SceneView, SceneViewMergeToParent,
};
use waterui::graphics::canvas::Canvas;
use waterui::graphics::color::Srgb;
use waterui::layout::scroll::ScrollView;
use waterui::text::Text;
use waterui_core::layout::{Point, Rect, Size};
use waterui_core::{AnyView, ContentLedger, Native, Signal, View};

#[derive(Debug)]
struct NoopDriver;

impl RuntimeDriver for NoopDriver {
    fn pump_at(&mut self, _at: std::time::Instant, _capture_snapshot: bool) -> DriverPumpResult {
        DriverPumpResult {
            rebuilt: false,
            profile: hydrolysis::FrameProfile::default(),
            tree_update: None,
            content_types: hydrolysis::AccessibilityContentTypes::new(),
            snapshot: None,
            device_lost: false,
            ui_focus: None,
        }
    }

    fn is_settled(&self) -> bool {
        true
    }

    fn is_settled_except_perpetual(&self) -> bool {
        true
    }

    fn has_pending_semantic_update(&self) -> bool {
        false
    }

    fn perform_accessibility_action(&mut self, _request: AccessibilityActionRequest) -> bool {
        false
    }

    fn push_input_event(&mut self, _event: InputEvent) {}

    fn request_redraw(&mut self) {}

    fn clear_ui_focus(&mut self) -> bool {
        false
    }

    fn wait_for_local_work(&self, _timeout: Duration) -> bool {
        false
    }
}

fn node_id(raw: u64) -> NodeId {
    NodeId::from(AccessibilityNodeId(raw))
}

fn node(
    id: u64,
    role: Role,
    label: Option<&str>,
    value: Option<&str>,
    enabled: bool,
) -> NodeSnapshot {
    NodeSnapshot {
        id: node_id(id),
        role,
        label: label.map(ToOwned::to_owned),
        identifier: None,
        value: value.map(ToOwned::to_owned),
        placeholder: None,
        bounds: None,
        enabled,
        selected: false,
        checked: None,
        expanded: None,
        busy: false,
        hidden: false,
        children: Vec::new(),
        actions: Vec::new(),
        scroll_x: None,
        scroll_y: None,
        content_type: None,
    }
}

fn tree(nodes: Vec<NodeSnapshot>) -> TreeSnapshot {
    let Some(root) = nodes.first().map(NodeSnapshot::id) else {
        panic!("test tree helper requires at least one node");
    };
    let nodes = nodes.into_iter().map(|node| (node.id(), node)).collect();
    TreeSnapshot {
        revision: 1,
        root,
        focus: root,
        nodes,
    }
}

fn scoped_tree() -> TreeSnapshot {
    let mut root = node(1, Role::LIST, Some("root"), None, true);
    root.children = vec![node_id(2), node_id(3)];

    let mut alpha = node(2, Role::LIST_ITEM, Some("Alpha card"), None, true);
    alpha.children = vec![node_id(4), node_id(5)];

    let mut beta = node(3, Role::LIST_ITEM, Some("Beta card"), None, true);
    beta.children = vec![node_id(6), node_id(7)];

    let edit_alpha = node(4, Role::BUTTON, Some("Edit"), None, true);
    let email_alpha = node(
        5,
        Role::TEXT_INPUT,
        Some("Email"),
        Some("alpha@example.com"),
        true,
    );
    let edit_beta = node(6, Role::BUTTON, Some("Edit"), None, true);
    let email_beta = node(
        7,
        Role::TEXT_INPUT,
        Some("Email"),
        Some("beta@example.com"),
        true,
    );

    tree(vec![
        root,
        alpha,
        beta,
        edit_alpha,
        email_alpha,
        edit_beta,
        email_beta,
    ])
}

fn mounted(tree: TreeSnapshot) -> SemanticApp<NoopDriver> {
    mounted_on(NoopDriver, tree)
}

fn mounted_on<R: RuntimeDriver>(runtime: R, tree: TreeSnapshot) -> SemanticApp<R> {
    SemanticApp {
        runtime,
        tree,
        ui_focus: None,
        revision: 2,
        viewport: (0, 0),
        clock: None,
        resources: ResourceSampler::new(),
        ledger: ContentLedger::new(),
    }
}

/// A driver whose `emit_at`-th pump emits `update` and reports settled from
/// that pump on — busy before, quiescent after.
///
/// This is the case a frame-bounded wait must not misread: the pump that
/// quiesces the runtime can itself carry the awaited node, so reporting
/// quiescence before re-evaluating would call a landed mount a timeout
/// (water-rs/waterui#2501).
struct EmitThenSettleDriver {
    pumps: usize,
    emit_at: usize,
    update: AccessibilityTreeUpdate,
}

impl RuntimeDriver for EmitThenSettleDriver {
    fn pump_at(&mut self, _at: std::time::Instant, _capture_snapshot: bool) -> DriverPumpResult {
        self.pumps += 1;
        DriverPumpResult {
            rebuilt: false,
            profile: hydrolysis::FrameProfile::default(),
            tree_update: (self.pumps == self.emit_at).then(|| self.update.clone()),
            content_types: hydrolysis::AccessibilityContentTypes::new(),
            snapshot: None,
            device_lost: false,
            ui_focus: None,
        }
    }

    fn is_settled(&self) -> bool {
        self.pumps >= self.emit_at
    }

    fn is_settled_except_perpetual(&self) -> bool {
        self.pumps >= self.emit_at
    }

    fn has_pending_semantic_update(&self) -> bool {
        false
    }

    fn perform_accessibility_action(&mut self, _request: AccessibilityActionRequest) -> bool {
        false
    }

    fn push_input_event(&mut self, _event: InputEvent) {}

    fn request_redraw(&mut self) {}

    fn clear_ui_focus(&mut self) -> bool {
        false
    }

    fn wait_for_local_work(&self, _timeout: Duration) -> bool {
        false
    }
}

/// One scripted capture frame: a snapshot, or a frame its device loss
/// skipped, or neither.
#[derive(Clone, Copy)]
enum CaptureFrame {
    Captured,
    DeviceLost,
    Empty,
}

/// A driver that answers each capture pump with the next scripted
/// [`CaptureFrame`] and records the instant and capture flag every pump got.
/// The headless runtime's own device-loss reporting is covered against a
/// real device in Hydrolysis (`generation_tests`); this checks what the
/// capture does with the report.
struct CaptureDriver {
    frames: std::collections::VecDeque<CaptureFrame>,
    pumps: Vec<(std::time::Instant, bool)>,
}

impl CaptureDriver {
    fn new(frames: impl IntoIterator<Item = CaptureFrame>) -> Self {
        Self {
            frames: frames.into_iter().collect(),
            pumps: Vec::new(),
        }
    }
}

impl RuntimeDriver for CaptureDriver {
    fn pump_at(&mut self, at: std::time::Instant, capture_snapshot: bool) -> DriverPumpResult {
        self.pumps.push((at, capture_snapshot));
        let frame = self
            .frames
            .pop_front()
            .expect("the capture pumped past its scripted frames");
        DriverPumpResult {
            rebuilt: false,
            profile: hydrolysis::FrameProfile::default(),
            tree_update: None,
            content_types: hydrolysis::AccessibilityContentTypes::new(),
            snapshot: matches!(frame, CaptureFrame::Captured)
                .then(|| Snapshot::new(1, 1, vec![255, 0, 0, 255])),
            device_lost: matches!(frame, CaptureFrame::DeviceLost),
            ui_focus: None,
        }
    }

    fn is_settled(&self) -> bool {
        true
    }

    fn is_settled_except_perpetual(&self) -> bool {
        true
    }

    fn has_pending_semantic_update(&self) -> bool {
        false
    }

    fn perform_accessibility_action(&mut self, _request: AccessibilityActionRequest) -> bool {
        false
    }

    fn push_input_event(&mut self, _event: InputEvent) {}

    fn request_redraw(&mut self) {}

    fn clear_ui_focus(&mut self) -> bool {
        false
    }

    fn wait_for_local_work(&self, _timeout: Duration) -> bool {
        false
    }
}

/// A capture whose frames lose their device re-renders at the same frame
/// instant until one carries a snapshot, so the capture neither fails on a
/// recoverable loss nor lands a frame later than it asked for.
#[test]
fn capture_rerenders_a_device_lost_frame_at_the_same_instant() {
    let mut app = mounted_on(
        CaptureDriver::new([
            CaptureFrame::DeviceLost,
            CaptureFrame::DeviceLost,
            CaptureFrame::Captured,
        ]),
        tree(vec![node(1, Role::GROUP, Some("root"), None, true)]),
    );
    let snapshot = app.capture_frame();
    assert_eq!((snapshot.width(), snapshot.height()), (1, 1));

    let pumps = &app.runtime.pumps;
    assert_eq!(pumps.len(), 3, "two lost frames, then the captured one");
    assert!(
        pumps
            .iter()
            .all(|&(at, capture)| at == pumps[0].0 && capture),
        "every attempt captures at the one frame instant"
    );
}

/// A capture frame that carries no snapshot without a device loss to
/// explain it fails at once, naming why.
#[test]
fn capture_without_a_snapshot_or_device_loss_fails_fast() {
    let mut app = mounted_on(
        CaptureDriver::new([CaptureFrame::Empty]),
        tree(vec![node(1, Role::GROUP, Some("root"), None, true)]),
    );
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app.capture_frame();
    }));
    let message = panic_message(&*outcome.expect_err("an empty capture frame must fail"));
    assert!(
        message.contains("did not produce a snapshot") && message.contains("no device loss"),
        "the failure names the missing snapshot and that no loss explains it: {message}"
    );
    assert_eq!(app.runtime.pumps.len(), 1, "no retry without a device loss");
}

/// A full-tree update carrying a `role`/`label` node under the root.
fn update_with_node(id: u64, role: AccessibilityRole, label: &str) -> AccessibilityTreeUpdate {
    let mut root = AccessibilityNode::new(AccessibilityRole::Group);
    root.set_children([AccessibilityNodeId(id)]);
    let mut child = AccessibilityNode::new(role);
    child.set_label(label);
    AccessibilityTreeUpdate {
        nodes: vec![
            (AccessibilityNodeId(1), root),
            (AccessibilityNodeId(id), child),
        ],
        tree: Some(AccessibilityTreeInfo::new(AccessibilityNodeId(1))),
        tree_id: AccessibilityTreeId::ROOT,
        focus: AccessibilityNodeId(1),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        return (*message).to_owned();
    }
    String::from("<non-string panic>")
}

/// A view that draws widget chrome mounts on the semantic pipeline: the scroll
/// view's container and the button's node are products of the view tree and
/// the widgets' semantics — no style package is installed (#290).
#[test]
fn scroll_view_and_button_structure_mounts_without_a_style() {
    let mut app = ui().viewport(240, 160).mount(|| {
        ScrollView::vertical(vstack((
            waterui::component::button("Submit"),
            text("Scrolled content").body(),
        )))
    });
    assert_eq!(app.query().role(Role::BUTTON).all().len(), 1);
    app.query()
        .role(Role::LABEL)
        .label("Scrolled content")
        .assert_exists();
}

#[test]
fn semantic_builder_does_not_require_theme_package() {
    let mut app = ui()
        .viewport(180, 80)
        .mount(|| text("Semantic only").body());
    let _ = app
        .query()
        .role(Role::LABEL)
        .label("Semantic only")
        .single();
}

/// The smallest `Style` a test can write: `install_tokens`
/// overrides the accent slot the framework defaults carry, and the
/// `WidgetTheme` surface `Style` requires answers with inert metrics and
/// no-op draw calls — nothing renders on the semantic pipeline.
mod token_probe {
    use std::time::Duration;

    use hydrolysis::theme::{
        BadgeMetrics, ButtonLook, ButtonSpec, Chrome, CornerShape, DividerMetrics, DragMapping,
        FocusIndicator, HitRegion, IndicatorLook, InteractionMotion, Knob, Label,
        LinearProgressLook, ListMetrics, MenuLook, MenuPickerLook, MenuSpec, Motion,
        NavigationBarLook, NavigationSpec, PickerSpec, PressFeedback, ProgressControl,
        ProgressSpec, RadioGroupLook, SliderLayout, SliderLook, SliderSpec, StateLayer,
        StepperLook, StepperSpec, Surface, TableMetrics, TabsLook, TabsSpec, TextFieldLook,
        TextFieldSpec, ToggleSpec, WidgetTheme,
    };
    use hydrolysis::theme::{CheckboxLook, SwitchDrag, SwitchLook};
    use kurbo::{Point, Rect};
    use waterui::animation::Animation;
    use waterui::color::Srgb;
    use waterui::component::text;
    use waterui::env::use_env;
    use waterui::graphics::backend::draw::Recorder;
    use waterui::reactive::constant;
    use waterui::text::font::Font;
    use waterui::theme::{color as theme_color, install_color_signal, installed_color_signal};
    use waterui::{Color, Environment, Signal as _, SignalExt as _, View};

    use crate::Style;

    /// The accent `TokenProbeStyle` installs — deliberately far from the
    /// framework default (`#2563EB`) so the two mounts cannot collide.
    pub(super) const PROBE_ACCENT: Srgb = Srgb::from_u32(0x00_99_66);
    /// The accent the framework defaults install (`install_default_tokens`).
    pub(super) const DEFAULT_ACCENT: Srgb = Srgb::from_u32(0x25_63_EB);

    /// Formats an accent the way [`accent_probe`] labels it.
    pub(super) fn accent_label(accent: Srgb) -> String {
        format!("accent:{:?}", accent.resolve())
    }

    /// Reads the environment's accent slot and publishes its resolved value
    /// as an `accent:<resolved>` label — the way a style-driven component
    /// body consumes a token during view build.
    pub(super) fn accent_probe() -> impl View {
        use_env(|env: Environment| {
            let accent = installed_color_signal::<theme_color::Accent>(&env)
                .expect("the framework defaults carry the accent slot")
                .snapshot();
            text(format!("accent:{accent:?}"))
        })
    }

    #[derive(Debug)]
    pub(super) struct TokenProbeStyle;

    impl Style for TokenProbeStyle {
        fn install_tokens(&self, env: &mut Environment) {
            install_color_signal::<theme_color::Accent>(
                env,
                constant(PROBE_ACCENT.resolve()).computed(),
            );
        }
    }

    impl WidgetTheme for TokenProbeStyle {
        fn interaction_motion(&self) -> InteractionMotion {
            InteractionMotion {
                hover_opacity: 0.08,
                focus_opacity: 0.12,
                pressed_opacity: 0.12,
                dragged_opacity: 0.16,
                hover_enter: Animation::linear(Duration::from_millis(15)),
                hover_exit: Animation::linear(Duration::from_millis(15)),
                focus_enter: Animation::linear(Duration::from_millis(15)),
                focus_exit: Animation::linear(Duration::from_millis(15)),
                press_fade_in: Animation::linear(Duration::from_millis(105)),
                press_fade_out: Animation::linear(Duration::from_millis(375)),
                press_grow: Animation::bezier(Duration::from_millis(450), 0.2, 0.0, 0.0, 1.0),
                press_shape: Animation::linear(Duration::from_millis(15)),
                minimum_press_duration: Duration::from_millis(225),
                touch_delay: Duration::from_millis(150),
            }
        }

        fn button(&self, _spec: &ButtonSpec, _env: &Environment) -> ButtonLook {
            let chrome = Chrome::new(
                Surface::clear(CornerShape::radius(0.0)),
                StateLayer::new(Color::new(theme_color::Foreground)),
                PressFeedback::new(),
                FocusIndicator::Tint,
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
            // The theme's `touch_delay`.
            .hit(HitRegion::NONE.press_delay(Duration::from_millis(150)));
            ButtonLook::new(chrome, Label::new())
                .padding(1.0, 2.0)
                .minimum_size(kurbo::Size::new(123.0, 45.0))
        }

        fn switch(&self, spec: &ToggleSpec, _env: &Environment) -> SwitchLook {
            SwitchLook::new(
                hydrolysis::testing::plain_toggle_control(),
                SwitchDrag::new(6.0, spec.direction()),
            )
        }

        fn checkbox(&self, _spec: &ToggleSpec, _env: &Environment) -> CheckboxLook {
            CheckboxLook::new(hydrolysis::testing::plain_toggle_control())
        }

        fn stepper(&self, _spec: &StepperSpec, _env: &Environment) -> StepperLook {
            hydrolysis::testing::plain_stepper_look()
        }

        fn text_field(&self, _spec: &TextFieldSpec, _env: &Environment) -> TextFieldLook {
            hydrolysis::testing::plain_text_field_look(
                hydrolysis::testing::plain_chrome(),
                hydrolysis::testing::plain_input_metrics(),
            )
        }

        fn menu(&self, _spec: &MenuSpec, _env: &Environment) -> MenuLook {
            hydrolysis::testing::plain_menu_look()
        }

        fn menu_picker(&self, _spec: &PickerSpec, _env: &Environment) -> MenuPickerLook {
            hydrolysis::testing::plain_menu_picker_look()
        }

        fn radio_group(&self, _spec: &PickerSpec, _env: &Environment) -> RadioGroupLook {
            hydrolysis::testing::plain_radio_group_look()
        }

        fn segmented_picker(
            &self,
            _spec: &hydrolysis::theme::SegmentedSpec,
            _env: &Environment,
        ) -> hydrolysis::theme::SegmentedLook {
            hydrolysis::testing::plain_segmented_look()
        }

        fn slider(&self, _spec: &SliderSpec, _env: &Environment) -> SliderLook {
            SliderLook::new(
                SliderLayout {
                    row_height: 44.0,
                    track_inset: 12.0,
                    label_inset: 12.0,
                    label_gap: 8.0,
                    minimum_track: 72.0,
                    label_spacing: Some(6.0),
                },
                Knob {
                    size: kurbo::Size::new(4.0, 44.0),
                    hit: HitRegion::NONE,
                },
                DragMapping::Absolute,
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            )
        }

        fn linear_progress(&self, _spec: &ProgressSpec, _env: &Environment) -> LinearProgressLook {
            LinearProgressLook::new(
                ProgressControl::new(
                    kurbo::Size::new(72.0, 8.0),
                    Motion::uniform(Animation::linear(Duration::from_millis(15))),
                ),
                8.0,
            )
            .label_band(18.0, 10.0)
            .value_label_band(6.0)
        }

        fn circular_progress(&self, _spec: &ProgressSpec, _env: &Environment) -> IndicatorLook {
            IndicatorLook::new(ProgressControl::new(
                kurbo::Size::new(32.0, 32.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            ))
        }

        fn loading_indicator(&self, _spec: &ProgressSpec, _env: &Environment) -> IndicatorLook {
            IndicatorLook::new(ProgressControl::new(
                kurbo::Size::new(48.0, 48.0),
                Motion::uniform(Animation::linear(Duration::from_millis(15))),
            ))
        }

        fn navigation_bar(&self, _spec: &NavigationSpec, _env: &Environment) -> NavigationBarLook {
            hydrolysis::testing::plain_navigation_look()
        }

        fn tabs(&self, _spec: &TabsSpec, _env: &Environment) -> TabsLook {
            hydrolysis::testing::plain_tabs_look()
        }

        fn draw_scroll_indicator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

        fn divider_metrics(&self) -> DividerMetrics {
            DividerMetrics { thickness: 1.0 }
        }

        fn draw_divider(&self, _recorder: &mut Recorder, _bounds: Rect) {}

        fn badge_metrics(&self) -> BadgeMetrics {
            BadgeMetrics {
                small_size: 6.0,
                large_size: 16.0,
                large_horizontal_padding: 4.0,
                small_offset_x: 6.0,
                small_offset_y: 6.0,
                large_offset_x: 12.0,
                large_offset_y: 14.0,
            }
        }

        fn badge_label_color(&self) -> Color {
            Color::srgb(255, 255, 255)
        }

        fn badge_label_font(&self) -> Font {
            Font::default()
        }

        fn draw_badge_small(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_badge_large(&self, _recorder: &mut Recorder, _bounds: Rect) {}

        fn list_metrics(&self) -> ListMetrics {
            ListMetrics {
                one_line_row_height: 56.0,
                horizontal_inset: 16.0,
                vertical_inset: 10.0,
                divider_leading_inset: 16.0,
                divider_trailing_inset: 16.0,
                move_control_width: 20.0,
                delete_control_width: 26.0,
                trailing_control_spacing: 6.0,
                trailing_control_vertical_inset: 6.0,
                section_header_height: 48.0,
                section_footer_height: 40.0,
            }
        }

        fn draw_list_row_background(
            &self,
            _recorder: &mut Recorder,
            _bounds: Rect,
            _alternate: bool,
        ) {
        }
        fn draw_list_move_control(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_list_delete_control(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_list_separator(&self, _recorder: &mut Recorder, _bounds: Rect) {}

        fn table_metrics(&self) -> TableMetrics {
            TableMetrics {
                min_column_width: 72.0,
                cell_horizontal_padding: 32.0,
                cell_vertical_inset: 16.0,
                header_height: 56.0,
                row_height: 52.0,
                outline_width: 1.0,
            }
        }

        fn draw_table_background(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_table_header_background(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_table_cell_border(&self, _recorder: &mut Recorder, _bounds: Rect) {}
        fn draw_table_column_separator(&self, _recorder: &mut Recorder, _from: Point, _to: Point) {}
    }
}

/// `ui().theme(style).mount(..)` stays on the semantic runtime but installs
/// the style's tokens into the mounted view's environment, above the
/// framework defaults; `ui().mount(..)` resolves the defaults only.
#[test]
fn styled_semantic_mount_installs_style_tokens_over_framework_defaults() {
    fn accent_label(app: &mut SemanticApp) -> String {
        app.query()
            .role(Role::LABEL)
            .single()
            .node()
            .label()
            .expect("the probe text mounts as a label")
            .to_owned()
    }

    let mut styled = ui()
        .theme(token_probe::TokenProbeStyle)
        .mount(token_probe::accent_probe);
    let styled_label = accent_label(&mut styled);

    let mut unstyled = ui().mount(token_probe::accent_probe);
    let unstyled_label = accent_label(&mut unstyled);

    assert_eq!(
        styled_label,
        token_probe::accent_label(token_probe::PROBE_ACCENT)
    );
    assert_eq!(
        unstyled_label,
        token_probe::accent_label(token_probe::DEFAULT_ACCENT)
    );
    assert_ne!(styled_label, unstyled_label);
}

/// `UiBuilder<Styled<S>>::mount_app` honours the builder's viewport and scale
/// factor over the window's declared frame: the window content is allocated
/// `320x240` logical points — not the `800x600` `Window::new` declares — and
/// the scale factor stays a capture concern, so bounds read back in logical
/// points. A text root reports its centred response by default
/// (`docs/layout-spec.md` §6), so the label names its allocation (§7.2) to
/// report the frame the mount assigned.
#[test]
fn styled_builder_mount_app_mounts_at_the_builder_viewport() {
    let app = waterui::app::App::new(
        || text("app content").a11y_geometry(AccessibilityGeometry::Allocation),
        waterui::Environment::new(),
    );
    let mut app = ui()
        .theme(token_probe::TokenProbeStyle)
        .viewport(320, 240)
        .scale_factor(2.0)
        .mount_app(app);
    let bounds = app.query().role(Role::LABEL).single().bounds();
    assert_eq!(bounds, NodeBounds::new(0.0, 0.0, 320.0, 240.0));
}

/// The free `mount_app(app, style)` convenience is the builder form with the
/// viewport sized from the window's declared frame — `Window::new`'s default
/// `800x600` here, not the builder's `390x844` default. The label names its
/// allocation (`docs/layout-spec.md` §7.2) so it reports that frame rather
/// than the text's centred response.
#[test]
fn free_mount_app_mounts_at_the_window_frame() {
    let app = waterui::app::App::new(
        || text("app content").a11y_geometry(AccessibilityGeometry::Allocation),
        waterui::Environment::new(),
    );
    let mut app = mount_app(app, token_probe::TokenProbeStyle);
    let bounds = app.query().role(Role::LABEL).single().bounds();
    assert_eq!(bounds, NodeBounds::new(0.0, 0.0, 800.0, 600.0));
}

/// Chained frame setters mutate one `FrameLayout`, so `.size(40, 20)` followed
/// by `.min_width(60)` inverts the width axis — and `min` wins, the same
/// precedence CSS gives a `min-width` over a conflicting `max-width`: the
/// chain asks for a 60x20 frame and resolves to exactly that.
#[test]
fn a_size_then_min_width_chain_resolves_to_the_minimum() {
    let mut app = ui()
        .theme(token_probe::TokenProbeStyle)
        .viewport(200, 200)
        .mount_offscreen(|| {
            vstack((waterui::Color::srgb_hex("#E67E22")
                .size(40.0, 20.0)
                .min_width(60.0)
                .a11y_label("chained"),))
        });
    let bounds = app.query().label("chained").single().bounds();
    assert_eq!((bounds.width(), bounds.height()), (60.0, 20.0));
}

/// `.size(50, 80)` with `.min_height(100)` and `.min_width(60)` inverts both
/// axes; each resolves to its minimum.
#[test]
fn a_size_then_min_height_and_min_width_chain_resolves_to_the_minimum() {
    let mut app = ui()
        .theme(token_probe::TokenProbeStyle)
        .viewport(200, 200)
        .mount_offscreen(|| {
            vstack((waterui::Color::srgb_hex("#E67E22")
                .size(50.0, 80.0)
                .min_height(100.0)
                .min_width(60.0)
                .a11y_label("chained"),))
        });
    let bounds = app.query().label("chained").single().bounds();
    assert_eq!((bounds.width(), bounds.height()), (60.0, 100.0));
}

#[test]
fn a11y_identifier_flows_from_modifier_to_selector() {
    let mut app = ui().mount(|| {
        vstack((
            waterui::component::button("Submit").a11y_id("login.submit"),
            waterui::component::button("Submit"),
        ))
    });
    let element = app.query().identifier("login.submit").single();
    assert_eq!(element.node().identifier(), Some("login.submit"));
    assert_eq!(element.node().label(), Some("Submit"));
    // The identifier is nearest-consumer metadata: the second, unadorned
    // button must not inherit it.
    assert_eq!(
        app.query().role(Role::BUTTON).all().len(),
        2,
        "both buttons stay queryable by role"
    );
    app.query().identifier("login.submit").tap();
}

/// A tap gesture with no naming scope of its own still publishes an
/// activatable element: the wrapped node takes the `Button` claim's `Click`
/// and `Focus` actions and its activation, so a tappable `text("T")` reads
/// the way a `Button` does instead of serializing identically to plain
/// text (water-rs/waterui#1070).
#[test]
fn tap_gesture_publishes_click_action_on_the_wrapped_node() {
    let taps = Rc::new(Cell::new(0_u32));
    let taps_for_view = Rc::clone(&taps);
    let mut app = ui().mount(move || {
        let taps_for_view = Rc::clone(&taps_for_view);
        vstack((
            text("T").on_tap(move || {
                taps_for_view.set(taps_for_view.get() + 1);
            }),
            text("Plain"),
        ))
    });

    let tappable = app.query().role(Role::BUTTON).label("T").single();
    assert!(
        tappable
            .node()
            .actions()
            .contains(&accesskit::Action::Click),
        "the wrapped node advertises Click"
    );
    tappable.tap(&mut app);
    assert_eq!(taps.get(), 1, "the click action runs the tap handler");

    // The untapped sibling stays inert — a plain label with no actions.
    let plain = app.query().role(Role::LABEL).label("Plain").single();
    assert_eq!(plain.node().actions(), []);
    assert_eq!(
        app.query().role(Role::BUTTON).all().len(),
        1,
        "only the tappable view claims a button element"
    );
}

/// A `TextField` prompt publishes the `aria-placeholder` semantic on the
/// field's own node (#1071): before the snapshot carried the property, a
/// field with a prompt and one without produced identical snapshots.
#[test]
fn text_field_prompt_publishes_placeholder_on_the_node() {
    use waterui::prelude::*;

    let email = Binding::container(Str::from(""));
    let plain = Binding::container(Str::from(""));
    let mut app = ui().mount(move || {
        vstack((
            TextField::new(text("Email"), &email).prompt("name@example.com"),
            TextField::new(text("Plain"), &plain),
        ))
    });

    let prompted = app.query().role(Role::TEXT_INPUT).label("Email").single();
    assert_eq!(prompted.node().placeholder(), Some("name@example.com"));
    // The placeholder is a first-class selector predicate, the way `label` and
    // `value` are.
    app.query()
        .role(Role::TEXT_INPUT)
        .placeholder("name@example.com")
        .assert_exists();

    let unprompted = app.query().role(Role::TEXT_INPUT).label("Plain").single();
    assert_eq!(unprompted.node().placeholder(), None);
}

/// Explicit `.color(...)` is a view-tree attribute: on the semantic pipeline
/// the two labels stay queryable with it set.
#[test]
fn explicit_text_color_preserves_semantic_labels() {
    let mut app = ui().viewport(240, 120).mount(|| {
        vstack((
            text("Explicit color").body().color(Srgb::WHITE),
            text("Explicit color").body().color(Srgb::WHITE),
        ))
        .background(Srgb::BLACK)
    });
    assert_eq!(
        app.query()
            .role(Role::LABEL)
            .label("Explicit color")
            .all()
            .len(),
        2,
        "explicit text color should not break semantic text exposure"
    );
}

#[test]
fn smoke_text_preserves_semantic_labels() {
    let mut app = ui().viewport(240, 120).mount(|| {
        vstack((
            text("Focused datum").body().foreground(Srgb::WHITE),
            text("Selected datum").body().foreground(Srgb::WHITE),
        ))
        .background(Srgb::BLACK)
    });
    app.query()
        .role(Role::LABEL)
        .label("Focused datum")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Selected datum")
        .assert_exists();
}

#[test]
fn tappable_composed_view_exposes_clickable_accessibility_node() {
    let tapped = Rc::new(Cell::new(false));
    let tapped_for_view = Rc::clone(&tapped);
    let mut app = ui().viewport(160, 96).mount(move || {
        text("Assist")
            .body()
            .padding_with(6.0)
            .on_tap({
                let tapped_for_view = Rc::clone(&tapped_for_view);
                move || tapped_for_view.set(true)
            })
            .a11y_label("Assist")
            .a11y_role(waterui::accessibility::AccessibilityRole::Button)
            .a11y_children(waterui::accessibility::AccessibilityChildren::ExcludeDescendants)
    });

    app.query()
        .role(Role::BUTTON)
        .label("Assist")
        .assert_exists();
    app.query().role(Role::BUTTON).label("Assist").tap();
    assert!(
        tapped.get(),
        "accessibility click should trigger tap gesture"
    );
    app.query()
        .role(Role::LABEL)
        .label("Assist")
        .assert_not_exists();
}

/// A `Canvas` composes to `CanvasView`, whose accessibility metadata is a
/// product of the view tree — the IMAGE role and its label exist on the
/// semantic pipeline with nothing rendered.
#[test]
fn canvas_and_text_expose_accessibility_nodes_semantically() {
    let mut app = ui().viewport(320, 320).mount(|| {
        vstack((
            Canvas::new(|cx| {
                cx.fill(
                    Rect::new(Point::new(0.0, 0.0), Size::new(240.0, 180.0)),
                    Srgb::new(0.0, 0.85, 0.65).resolve(),
                );
            })
            .size(240.0, 180.0)
            .a11y_role(waterui::accessibility::AccessibilityRole::Image)
            .a11y_label("Canvas layer"),
            text("W")
                .size(48.0)
                .color(Srgb::WHITE)
                .body()
                .padding_with(6.0)
                .a11y_label("Letter W"),
        ))
        .spacing(6.0)
        .background(Srgb::BLACK)
    });
    app.query()
        .role(Role::IMAGE)
        .label("Canvas layer")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("Letter W")
        .assert_exists();
}

/// A `SceneView` marked to merge exposes its accessibility node on the
/// semantic pipeline: the role and label are view-tree metadata, the scene's
/// pixels are the rendered pipeline's concern.
#[test]
fn scene_view_exposes_accessibility_node_semantically() {
    let mut app = ui().viewport(96, 72).mount(|| {
        SceneView::new(TestSceneContent(Rc::new(Cell::new(false))))
            .a11y_role(waterui::accessibility::AccessibilityRole::Image)
            .a11y_label("Scene layer")
    });
    app.query()
        .role(Role::IMAGE)
        .label("Scene layer")
        .assert_exists();
}

struct TestSceneContent(Rc<Cell<bool>>);

impl SceneContent for TestSceneContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        self.0.set(true);
        let rect = kurbo::Rect::from_origin_size(
            (8.0, 8.0),
            (f64::from(width.min(40.0)), f64::from(height.min(24.0))),
        );
        let red = draw::Color::<draw::Srgb>::new([1.0, 0.0, 0.0, 1.0]);
        recorder.fill(rect, WorkingColor::from(red));
        NextFrame::Idle
    }

    fn rebuild_for_engine(&mut self) {}
}

#[test]
fn scene_view_body_merges_to_native_when_marker_is_present() {
    let env = waterui_core::Environment::new().extending(SceneViewMergeToParent);
    let body = SceneView::new(TestSceneContent(Rc::new(Cell::new(false)))).body(&env);
    let any = AnyView::new(body);
    assert!(
        any.is::<Native<SceneView>>(),
        "expected SceneView body to resolve to Native<SceneView> when merge marker is present"
    );
}

/// `spawn_local` work scheduled from `on_appear` drains on the semantic
/// pipeline too — the parked-task executor is runtime-agnostic.
#[test]
fn semantic_mount_drains_spawned_local_work() {
    use waterui::task::spawn_local;
    use waterui::{Binding, ViewExt as _};

    let status = Binding::container(String::from("idle"));
    let status_for_view = status.clone();

    let mut app = ui().mount(move || {
        waterui::text!("{status_for_view}")
            .on_appear(|status: waterui::State<Binding<String>>| {
                spawn_local(async move {
                    status.set(String::from("ready"));
                })
                .detach();
            })
            .state(&status_for_view)
    });

    let status_selector = Selector::default().role(Role::LABEL).label("ready");
    assert!(
        app.wait_for_existence(&status_selector, Duration::from_millis(500)),
        "expected the semantic runtime to drain spawn_local task and update the binding"
    );
    assert_eq!(status.snapshot().as_str(), "ready");
}

#[test]
fn query_chain_and_index_are_type_safe() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Save changes"), None, true),
        node(3, Role::BUTTON, Some("Save draft"), None, false),
    ]));

    let results = app
        .query()
        .role(Role::BUTTON)
        .label_contains("Save")
        .enabled(true)
        .all();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id().as_u64(), 2);
    assert_eq!(
        results[results[0].id()].node().label(),
        Some("Save changes")
    );
    assert_eq!(app.tree()[node_id(2)].label(), Some("Save changes"));
}

#[test]
fn hidden_nodes_are_excluded_unless_requested() {
    let mut hidden = node(3, Role::BUTTON, Some("Hidden action"), None, true);
    hidden.hidden = true;

    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Visible action"), None, true),
        hidden,
    ]));

    app.query()
        .role(Role::BUTTON)
        .label("Visible action")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Hidden action")
        .assert_not_exists();

    let hidden_match = app
        .query()
        .role(Role::BUTTON)
        .label("Hidden action")
        .hidden(true)
        .single();
    assert_eq!(hidden_match.id().as_u64(), 3);
}

#[test]
fn relative_queries_scope_by_semantic_handle() {
    let mut app = mounted(scoped_tree());

    let alpha = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Alpha card")
        .single();
    let beta = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Beta card")
        .single();

    let alpha_button = app
        .query()
        .within(&alpha)
        .role(Role::BUTTON)
        .label("Edit")
        .single();
    let alpha_input = app
        .query()
        .children_of(&alpha)
        .role(Role::TEXT_INPUT)
        .label("Email")
        .single();
    let beta_button = app
        .query()
        .within(&beta)
        .role(Role::BUTTON)
        .label("Edit")
        .single();

    assert_eq!(alpha_button.id().as_u64(), 4);
    assert_eq!(alpha_input.id().as_u64(), 5);
    assert_eq!(beta_button.id().as_u64(), 6);
}

#[test]
fn value_contains_matches_semantic_values() {
    let mut app = mounted(scoped_tree());

    let alpha_email = app
        .query()
        .role(Role::TEXT_INPUT)
        .value_contains("alpha@")
        .single();

    assert_eq!(alpha_email.id().as_u64(), 5);
}

#[test]
fn mixed_and_busy_selectors_preserve_complete_accessibility_state() {
    let mut state = node(2, Role::CHECKBOX, Some("Sync all"), None, true);
    state.checked = Some(CheckedState::Mixed);
    state.busy = true;
    let mut app = mounted(tree(vec![
        node(1, Role::GROUP, Some("root"), None, true),
        state,
    ]));

    let element = app.query().role(Role::CHECKBOX).mixed().busy(true).single();

    assert_eq!(element.node().checked_state(), Some(CheckedState::Mixed));
    assert!(element.node().busy());
}

#[test]
fn wait_for_existence_and_nonexistence_complete_immediately() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::LABEL, Some("status"), Some("ready"), true),
    ]));

    let status_selector = Selector::default().role(Role::LABEL).label("status");
    let missing_button_selector = Selector::default().role(Role::BUTTON).label("missing");
    assert!(app.wait_for_existence(&status_selector, Duration::from_millis(50),));
    assert!(app.wait_for_nonexistence(&missing_button_selector, Duration::from_millis(50),));
    assert!(app.wait_for_value_eq(&status_selector, "ready", Duration::from_millis(50),));
}

#[test]
fn wait_for_inverted_reports_fulfillment() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Delete"), None, true),
    ]));

    let expectation = app
        .expect_exists(Selector::default().role(Role::BUTTON).label("Delete"))
        .inverted();
    let result = app.wait_for(&[expectation], WaitOptions::new(Duration::from_millis(10)));
    assert_eq!(result, WaitResult::InvertedFulfillment);
}

#[test]
fn wait_for_times_out_when_condition_never_matches() {
    let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));

    let expectation = app.expect_exists(Selector::default().role(Role::BUTTON).label("never"));
    let result = app.wait_for(&[expectation], WaitOptions::new(Duration::from_millis(10)));
    assert_eq!(result, WaitResult::TimedOut);
}

#[test]
fn wait_for_panics_on_empty_expectations() {
    let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.wait_for(&[], WaitOptions::default());
    }));
    assert!(outcome.is_err());
}

#[test]
fn wait_for_ordered_expectations_skip_inverted_positions() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::LABEL, Some("Ready"), None, true),
        node(3, Role::BUTTON, Some("Continue"), None, true),
    ]));

    // An inverted expectation holds no position in the required order, so the
    // two present elements fulfill in list order and the wait completes.
    let expectations = [
        app.expect_exists(Selector::default().role(Role::BUTTON).label("Delete"))
            .inverted(),
        app.expect_exists(Selector::default().role(Role::LABEL).label("Ready")),
        app.expect_exists(Selector::default().role(Role::BUTTON).label("Continue")),
    ];
    let result = app.wait_for(
        &expectations,
        WaitOptions::new(Duration::from_millis(10)).enforce_order(true),
    );
    assert_eq!(result, WaitResult::Completed);
}

#[test]
fn wait_for_frames_completes_when_the_expectation_is_met() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::IMAGE, Some("Video content"), None, true),
    ]));

    let motion = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(app.wait_for_frames(&[motion], 4), WaitResult::Completed);
}

#[test]
fn wait_for_frames_stops_when_a_quiescent_runtime_cannot_produce_the_node() {
    let mut app = mounted(tree(vec![node(1, Role::LIST, Some("root"), None, true)]));

    // `NoopDriver` is settled from the start: with nothing in flight the wait
    // must not burn its frame budget before answering TimedOut.
    let missing = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(app.wait_for_frames(&[missing], 60), WaitResult::TimedOut);
}

#[test]
fn wait_for_frames_observes_the_pump_that_settled_the_runtime() {
    // The mount lands inside the pump that quiesces the runtime — the
    // sequence behind water-rs/waterui#2501: the press's gesture deadline
    // fires, `is_playing` flips, the `Dynamic` rebuild mounts the motion and
    // the window goes idle in that same frame. A wait that reported
    // quiescence before re-evaluating would call this a timeout.
    let update = update_with_node(2, AccessibilityRole::Image, "Video content");
    let mut app = mounted_on(
        EmitThenSettleDriver {
            pumps: 0,
            emit_at: 2,
            update,
        },
        tree(vec![node(1, Role::LIST, Some("root"), None, true)]),
    );

    let motion = app.expect_exists(Selector::default().role(Role::IMAGE).label("Video content"));
    assert_eq!(app.wait_for_frames(&[motion], 60), WaitResult::Completed);
}

#[test]
fn a_focus_only_update_keeps_the_tree() {
    // An update without `tree` is incremental (water-rs/waterui#2940): a
    // focus move carries no nodes, and the tree it lands on is unchanged.
    let focus_move = AccessibilityTreeUpdate {
        nodes: Vec::new(),
        tree: None,
        tree_id: AccessibilityTreeId::ROOT,
        focus: AccessibilityNodeId(3),
    };
    let mut app = mounted_on(
        EmitThenSettleDriver {
            pumps: 0,
            emit_at: 1,
            update: focus_move,
        },
        tree(vec![
            node(1, Role::LIST, Some("root"), None, true),
            node(2, Role::BUTTON, Some("Save"), None, true),
            node(3, Role::BUTTON, Some("Cancel"), None, true),
        ]),
    );

    app.settle();

    assert!(app.query().role(Role::BUTTON).label("Save").exists());
    assert_eq!(app.tree.root, node_id(1));
    assert_eq!(app.tree.focus, node_id(3));
}

#[test]
fn query_exists_is_true_for_multiple_matches() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("A"), None, true),
        node(3, Role::BUTTON, Some("A"), None, true),
    ]));

    assert!(app.query().role(Role::BUTTON).label("A").exists());
    assert!(!app.query().role(Role::BUTTON).label("B").exists());
}

#[test]
fn assert_ui_focus_failure_names_the_actual_focus_target() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("Save"), None, true),
        node(3, Role::BUTTON, Some("Cancel"), None, true),
    ]));
    app.ui_focus = Some(node_id(3));

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.assert_ui_focus(&Selector::default().role(Role::BUTTON).label("Save"));
    }));
    let message = panic_message(&*outcome.expect_err("assertion must fail"));
    assert!(
        message.contains("Cancel"),
        "failure must name the actual focus target: {message}"
    );
}

#[test]
fn query_optional_panics_on_multiple_matches() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("A"), None, true),
        node(3, Role::BUTTON, Some("A"), None, true),
    ]));

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app.query().role(Role::BUTTON).label("A").optional();
    }));
    assert!(outcome.is_err());
}

#[test]
fn element_set_index_by_node_id_panics_when_missing() {
    let mut app = mounted(tree(vec![
        node(1, Role::LIST, Some("root"), None, true),
        node(2, Role::BUTTON, Some("A"), None, true),
    ]));
    let set = app.query().role(Role::BUTTON).all();

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = &set[node_id(99)];
    }));
    assert!(outcome.is_err());
}

#[test]
fn stale_handle_panics_for_interaction_and_relative_query() {
    let mut app = mounted(scoped_tree());

    let alpha = app
        .query()
        .role(Role::LIST_ITEM)
        .label("Alpha card")
        .single();
    let edit = app
        .query()
        .within(&alpha)
        .role(Role::BUTTON)
        .label("Edit")
        .single();
    app.tree.revision = 99;

    let interaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        edit.tap(&mut app);
    }));
    let interaction_payload = interaction.expect_err("stale handle should panic");
    let interaction_message = panic_message(&*interaction_payload);
    assert!(
        interaction_message.contains("stale element handle"),
        "unexpected stale interaction panic: {interaction_message}"
    );

    let scoped_query = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app
            .query()
            .within(&alpha)
            .role(Role::BUTTON)
            .label("Edit")
            .single();
    }));
    let scoped_query_payload = scoped_query.expect_err("stale scoped query should panic");
    let scoped_query_message = panic_message(&*scoped_query_payload);
    assert!(
        scoped_query_message.contains("stale element handle"),
        "unexpected stale scoped query panic: {scoped_query_message}"
    );
}

#[test]
fn ui_focus_is_separate_from_accessibility_focus() {
    use waterui::form::secure::Secure;
    use waterui::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Username,
        Password,
    }

    let focus = Binding::container(Some(Field::Username));
    let username = Binding::container(Str::from(""));
    let password = Binding::container(Secure::default());
    let focus_for_view = focus.clone();
    let mut app = ui().mount(move || {
        vstack((
            TextField::new(text("Username"), &username).focused(&focus_for_view, Field::Username),
            SecureField::new(text("Password"), &password).focused(&focus_for_view, Field::Password),
            button("Submit"),
        ))
    });

    let username_selector = Selector::default().role(Role::TEXT_INPUT).label("Username");
    let password_selector = Selector::default()
        .role(Role::PASSWORD_INPUT)
        .label("Password");

    assert!(
        app.wait_for_ui_focus(&username_selector, Duration::from_millis(200)),
        "expected initial FocusState to focus the username field"
    );
    app.assert_ui_focus(&username_selector);
    assert_eq!(focus.snapshot(), Some(Field::Username));

    let username_id = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Username")
        .single()
        .id();
    assert_eq!(app.ui_focus(), Some(username_id));

    app.query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .focus();
    let password_id = app
        .query()
        .role(Role::PASSWORD_INPUT)
        .label("Password")
        .single()
        .id();
    app.assert_ui_focus(&password_selector);
    assert_eq!(app.ui_focus(), Some(password_id));
    assert_eq!(focus.snapshot(), Some(Field::Password));

    app.query().role(Role::BUTTON).label("Submit").focus();
    let submit_id = app.query().role(Role::BUTTON).label("Submit").single().id();
    assert_eq!(submit_id, app.tree().focus());
    assert_eq!(app.ui_focus(), Some(password_id));
    assert_eq!(focus.snapshot(), Some(Field::Password));

    app.clear_ui_focus();
    assert_eq!(app.ui_focus(), None);
    assert_eq!(focus.snapshot(), None);
    assert_eq!(app.tree().focus(), submit_id);
}

#[test]
fn runtime_focus_writes_move_and_clear_ui_focus() {
    use waterui::form::secure::Secure;
    use waterui::prelude::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Username,
        Password,
    }

    let focus = Binding::container(None::<Field>);
    let username = Binding::container(Str::from(""));
    let password = Binding::container(Secure::default());
    let focus_for_view = focus.clone();
    let mut app = ui().mount(move || {
        vstack((
            TextField::new(text("Username"), &username).focused(&focus_for_view, Field::Username),
            SecureField::new(text("Password"), &password).focused(&focus_for_view, Field::Password),
        ))
    });

    let username_selector = Selector::default().role(Role::TEXT_INPUT).label("Username");
    let password_selector = Selector::default()
        .role(Role::PASSWORD_INPUT)
        .label("Password");

    assert_eq!(app.ui_focus(), None);

    focus.set(Some(Field::Password));
    assert!(
        app.wait_for_ui_focus(&password_selector, Duration::from_millis(200)),
        "a runtime write to the focus binding must move UI focus to the password field"
    );
    app.assert_ui_focus(&password_selector);

    focus.set(Some(Field::Username));
    assert!(
        app.wait_for_ui_focus(&username_selector, Duration::from_millis(200)),
        "a later write must move UI focus back to the username field"
    );

    focus.set(None);
    app.settle();
    assert_eq!(app.ui_focus(), None);
}

#[test]
fn ui_focus_accepts_a_new_target_after_being_cleared() {
    use waterui::prelude::*;

    let focus = Binding::container(None::<i32>);
    let value = Binding::container(Str::from(""));
    let focus_for_view = focus.clone();
    let mut app =
        ui().mount(move || TextField::new(text("Field"), &value).focused(&focus_for_view, 0));

    let selector = Selector::default().role(Role::TEXT_INPUT).label("Field");

    focus.set(Some(0));
    assert!(app.wait_for_ui_focus(&selector, Duration::from_millis(200)));

    app.clear_ui_focus();
    assert_eq!(app.ui_focus(), None);
    assert_eq!(focus.snapshot(), None);

    app.query().role(Role::TEXT_INPUT).label("Field").focus();
    app.assert_ui_focus(&selector);
    assert_eq!(focus.snapshot(), Some(0));
}

#[test]
#[should_panic(expected = "requires exactly one TextField or SecureField")]
fn focused_modifier_without_a_text_anchor_panics() {
    use waterui::prelude::*;

    let focus = Binding::container(None::<i32>);
    let _app = ui().mount(move || button("No anchor").focused(&focus, 0));
}

#[test]
#[should_panic(expected = "found 2")]
fn focused_modifier_with_two_text_anchors_panics() {
    use waterui::prelude::*;

    let focus = Binding::container(None::<i32>);
    let first = Binding::container(Str::from(""));
    let second = Binding::container(Str::from(""));
    let _app = ui().mount(move || {
        vstack((
            TextField::new(text("First"), &first),
            TextField::new(text("Second"), &second),
        ))
        .focused(&focus, 0)
    });
}

#[test]
#[should_panic(expected = "multiple .focused()")]
fn focused_modifier_twice_on_the_same_control_panics() {
    use waterui::prelude::*;

    let focus_a = Binding::container(None::<i32>);
    let focus_b = Binding::container(None::<i32>);
    let value = Binding::container(Str::from(""));
    let _app = ui().mount(move || {
        TextField::new(text("Field"), &value)
            .focused(&focus_a, 0)
            .focused(&focus_b, 1)
    });
}

#[test]
fn committed_text_keeps_the_caret_at_the_end_across_retained_refreshes() {
    use waterui::prelude::*;

    let value = Binding::container(Str::from(""));
    let value_for_view = value.clone();
    let mut app = ui().mount(move || TextField::new(text("Full Name"), &value_for_view));

    app.query()
        .role(Role::TEXT_INPUT)
        .label("Full Name")
        .focus();

    let mut expected = String::new();
    for character in "Lexo Liu".chars() {
        expected.push(character);
        app.text_input(character.to_string());
        assert_eq!(
            value.snapshot().as_str(),
            expected,
            "each retained refresh must preserve the caret after the committed prefix"
        );
    }
}

/// A query answers about the app's state now, not as of the last interaction.
///
/// Every input path settles after dispatching, so a tap's consequences are in
/// the tree by the time the call returns. State a test changes directly — a
/// `Binding` it owns, set the way app code would — goes through no such path.
/// Without the sync on read, the next query answered from the tree as it stood
/// before the change: it reported the old label, and an assertion that should
/// have failed passed.
#[test]
fn a_query_sees_state_changed_since_the_last_pump() {
    let label = waterui::reactive::binding(waterui::Str::from("before"));
    let probe = label.clone();
    let mut app = crate::ui().mount(move || vstack((Text::computed(label.clone()),)));

    app.query()
        .role(crate::Role::LABEL)
        .label("before")
        .assert_exists();

    probe.set(waterui::Str::from("after"));

    app.query()
        .role(crate::Role::LABEL)
        .label("after")
        .assert_exists();
    app.query()
        .role(crate::Role::LABEL)
        .label("before")
        .assert_not_exists();
}

/// An app whose only activity is visual-only repaint still answers queries
/// promptly.
///
/// An indeterminate indicator repaints forever on a rendered runtime, but that
/// work never moves semantic state — so the semantic runtime settles with the
/// indicator on screen, and a query never waits on a pump budget it cannot
/// satisfy. What queries wait on is *unapplied* work, which is the state the
/// app leaves whenever a binding changes.
#[test]
fn a_visually_animating_app_still_settles_and_stays_current() {
    let label = waterui::reactive::binding(waterui::Str::from("before"));
    let probe = label.clone();
    let mut app = crate::ui().mount(move || {
        vstack((
            waterui::component::progress::loading().label("Loading"),
            Text::computed(label.clone()),
        ))
    });

    assert!(
        app.runtime.is_settled(),
        "an indeterminate indicator repaints forever but moves no semantic state — the semantic runtime settles"
    );
    assert!(
        !app.runtime.has_pending_semantic_update(),
        "with nothing unapplied the tree is current"
    );

    probe.set(waterui::Str::from("after"));
    assert!(
        app.runtime.has_pending_semantic_update(),
        "a signal change leaves an update the last flush did not apply"
    );

    app.query()
        .role(crate::Role::LABEL)
        .label("after")
        .assert_exists();
    assert!(
        !app.runtime.has_pending_semantic_update(),
        "reading the tree must have applied the update, not merely waited for it"
    );
}

/// A `press_named_key` stroke means the same thing on every runtime: the
/// semantic pipeline activates the focused control on the press and the
/// rendered pipeline on the release, so a focused button's action runs
/// identically under `mount` and `mount_offscreen`. water-rs/waterui#1222.
#[test]
fn named_key_stroke_activates_a_focused_button_on_both_runtimes() {
    use waterui::prelude::*;

    let count = Binding::i32(0);
    let count_for_view = count.clone();
    let mut app = ui().viewport(160, 96).mount(move || {
        waterui::component::button("Increment")
            .action(|waterui::State(count): waterui::State<Binding<i32>>| {
                *count.get_mut() += 1;
            })
            .state(&count_for_view)
    });
    app.query().role(Role::BUTTON).label("Increment").focus();
    app.press_named_key("Enter");
    assert_eq!(
        count.snapshot(),
        1,
        "Enter on a focused button must run its action on the semantic runtime"
    );

    let count = Binding::i32(0);
    let count_for_view = count.clone();
    let mut app = ui()
        .theme(token_probe::TokenProbeStyle)
        .viewport(160, 96)
        .mount_offscreen(move || {
            waterui::component::button("Increment")
                .action(|waterui::State(count): waterui::State<Binding<i32>>| {
                    *count.get_mut() += 1;
                })
                .state(&count_for_view)
        });
    app.query().role(Role::BUTTON).label("Increment").focus();
    app.press_named_key("Enter");
    assert_eq!(
        count.snapshot(),
        1,
        "Enter on a focused button must run its action on the rendered runtime"
    );
}

// ============================================================================
// List keyboard navigation (water-rs/waterui#1223)
// ============================================================================

/// Asserts the semantic tree's keyboard focus sits on the `List` row
/// labelled `Row {index}` — row navigation moves `accessibility.focus`,
/// which is what `tree().focus()` reports (UI focus is the separate
/// text-caret channel).
fn assert_row_focus(app: &mut SemanticApp, index: i32) {
    let id = app
        .query()
        .role(Role::LIST_ITEM)
        .label(format!("Row {index}"))
        .single()
        .id();
    assert_eq!(
        app.tree().focus(),
        id,
        "expected accessibility focus on Row {index}"
    );
}

/// Locates the focused row by position among the list's `ListItem`
/// children — for rows whose node does not carry the row label itself.
fn assert_row_focus_at(app: &mut SemanticApp, index: usize) {
    let list = app.query().role(Role::LIST).single();
    let rows = app.query().role(Role::LIST_ITEM).children_of(&list).all();
    let id = rows[index].id();
    assert_eq!(
        app.tree().focus(),
        id,
        "expected accessibility focus on row {index}"
    );
}

/// `List` rows answer `ArrowDown`/`ArrowUp` through the accessibility tree:
/// the arrows move keyboard focus to the adjacent row, the same focus the
/// pointer and Tab paths share.
#[test]
fn list_arrow_keys_move_row_focus() {
    let mut app = ui().mount(|| {
        List::content(
            (0..4)
                .map(|index| move || ListItem::new(text(format!("Row {index}"))))
                .collect::<Vec<_>>(),
        )
    });

    app.press_named_key("Tab");
    assert_row_focus(&mut app, 0);
    app.press_named_key("ArrowDown");
    assert_row_focus(&mut app, 1);
    app.press_named_key("ArrowDown");
    assert_row_focus(&mut app, 2);
    app.press_named_key("ArrowUp");
    assert_row_focus(&mut app, 1);
}

/// Arrow navigation only moves focus and scrolls: a `List` owns no
/// selection for the backend to write — selection is app state a row reads
/// through `ListItem::selected` — so stepping must never run a row's
/// activation, or an `on_tap` that opens, deletes, or navigates would fire
/// on every arrow press.
#[test]
fn list_arrow_keys_do_not_activate_rows() {
    let taps = Binding::container(0i32);
    let mut app = ui().mount({
        let taps = taps.clone();
        move || {
            List::content(
                (0..4)
                    .map(|index| {
                        let taps = taps.clone();
                        move || {
                            ListItem::new(text(format!("Row {index}")).on_tap({
                                let taps = taps.clone();
                                move || taps.with_mut(|t| *t += 1)
                            }))
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        }
    });
    app.press_named_key("Tab");
    assert_row_focus(&mut app, 0);
    app.press_named_key("ArrowDown");
    app.press_named_key("ArrowDown");
    app.press_named_key("ArrowUp");
    app.press_named_key("End");
    app.press_named_key("ArrowDown");
    app.press_named_key("Home");
    assert_row_focus(&mut app, 0);
    assert_eq!(
        taps.snapshot(),
        0,
        "arrow, Home and End moved focus without running any row action"
    );
}

/// `Enter`/`Space` activate the focused row through the activation it
/// already exposes — here the row's tap surfaced as a `Button` child —
/// exactly once per press, the way a pointer click on the row's centre
/// resolves.
#[test]
fn list_enter_activates_focused_row() {
    let taps = Binding::container(0i32);
    let mut app = ui().mount({
        let taps = taps.clone();
        move || {
            List::content(
                (0..4)
                    .map(|index| {
                        let taps = taps.clone();
                        move || {
                            ListItem::new(vstack((text(format!("Row {index}"))
                                .on_tap({
                                    let taps = taps.clone();
                                    move || taps.with_mut(|t| *t += 1)
                                })
                                .a11y_role(waterui::accessibility::AccessibilityRole::Button),)))
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        }
    });
    app.press_named_key("Tab");
    app.press_named_key("ArrowDown");
    assert_row_focus_at(&mut app, 1);
    app.press_named_key("Enter");
    assert_eq!(
        taps.snapshot(),
        1,
        "Enter runs the focused row's action once"
    );
    app.press_named_key("Space");
    assert_eq!(
        taps.snapshot(),
        2,
        "Space runs the focused row's action once"
    );
    assert_row_focus_at(&mut app, 1);
}

/// Stepping reveals the destination row: the semantic list reports its
/// scroll offset in row units (one unit per row), so a reveal of row N lands
/// `scrollY` at N.
#[test]
fn list_arrow_key_scrolls_row_into_view() {
    let mut app = ui().mount(|| {
        List::content(
            (0..20)
                .map(|index| move || ListItem::new(text(format!("Row {index}"))))
                .collect::<Vec<_>>(),
        )
    });

    app.press_named_key("Tab");
    assert_eq!(
        app.query().role(Role::LIST).single().node().scroll_y(),
        Some(0.0),
        "the list starts unscrolled"
    );
    app.press_named_key("ArrowDown");
    assert_eq!(
        app.query().role(Role::LIST).single().node().scroll_y(),
        Some(1.0),
        "stepping to row 1 scrolls it into view"
    );
    for _ in 0..5 {
        app.press_named_key("ArrowDown");
    }
    assert_eq!(
        app.query().role(Role::LIST).single().node().scroll_y(),
        Some(6.0),
        "stepping to row 6 scrolls it into view"
    );
    app.press_named_key("ArrowUp");
    assert_eq!(
        app.query().role(Role::LIST).single().node().scroll_y(),
        Some(5.0),
        "stepping back up scrolls the row into view again"
    );
}

/// `Home`/`End` move the focused row to the first and last rows of the list.
#[test]
fn list_home_end_move_row_focus_to_edges() {
    let mut app = ui().mount(|| {
        List::content(
            (0..4)
                .map(|index| move || ListItem::new(text(format!("Row {index}"))))
                .collect::<Vec<_>>(),
        )
    });
    app.press_named_key("Tab");
    assert_row_focus(&mut app, 0);
    app.press_named_key("ArrowDown");
    app.press_named_key("End");
    assert_row_focus(&mut app, 3);
    app.press_named_key("Home");
    assert_row_focus(&mut app, 0);
}

/// The arrows stop at the list's edges: stepping past the first or last row
/// keeps focus where it was rather than leaving the list.
#[test]
fn list_arrow_keys_stop_at_row_boundaries() {
    let mut app = ui().mount(|| {
        List::content(
            (0..3)
                .map(|index| move || ListItem::new(text(format!("Row {index}"))))
                .collect::<Vec<_>>(),
        )
    });
    app.press_named_key("Tab");
    assert_row_focus(&mut app, 0);
    app.press_named_key("ArrowUp");
    assert_row_focus(&mut app, 0);
    app.press_named_key("End");
    assert_row_focus(&mut app, 2);
    app.press_named_key("ArrowDown");
    assert_row_focus(&mut app, 2);
}
