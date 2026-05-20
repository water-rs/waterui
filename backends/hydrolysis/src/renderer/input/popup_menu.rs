use super::*;
use core::time::Duration;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::theme::color::{Surface, SurfaceVariant};
use waterui_controls::label::LabelDisplayMode;
use waterui_core::{AnimationExt as _, id::Id};
use waterui_layout::frame::Frame;
use waterui_layout::padding::EdgeInsets;

#[derive(Clone)]
pub(crate) struct ContextMenuTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) items: nami::Computed<Vec<ResolvedMenuItem>>,
}

#[derive(Clone)]
pub(crate) enum PopupMenuNode {
    Command {
        label: SemanticLabel,
        plain_label: String,
        action: SharedAction<()>,
        disabled: bool,
    },
    Divider,
    Menu {
        label: SemanticLabel,
        plain_label: String,
        items: Vec<PopupMenuNode>,
    },
}

#[derive(Clone)]
pub(crate) struct PopupMenuStateGroup(pub(crate) Rc<RefCell<Vec<Binding<WindowState>>>>);

#[derive(Default)]
pub(crate) struct PopupMenuState {
    pub(crate) active_popup_menu_group: Option<PopupMenuStateGroup>,
    pub(crate) picker_menu_slots: Vec<PickerMenuSlot>,
    pub(crate) picker_menu_cursor: usize,
}

pub(crate) struct PickerMenuSlot {
    pub(crate) open: Rc<Cell<bool>>,
}

#[derive(Clone)]
pub(crate) struct PickerMenuEntry {
    pub(crate) label: String,
    pub(crate) tag: Id,
}

impl PopupMenuState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.picker_menu_cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        for slot in self.picker_menu_slots.drain(self.picker_menu_cursor..) {
            slot.open.set(false);
        }
    }

    pub(crate) fn bind_picker_menu_state(&mut self) -> Rc<Cell<bool>> {
        let index = self.picker_menu_cursor;
        self.picker_menu_cursor = self
            .picker_menu_cursor
            .checked_add(1)
            .expect("picker menu slot cursor overflow");
        if index == self.picker_menu_slots.len() {
            self.picker_menu_slots.push(PickerMenuSlot::new());
        }
        Rc::clone(&self.picker_menu_slots[index].open)
    }
}

impl PopupMenuStateGroup {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    pub(crate) fn push(&self, state: Binding<WindowState>) {
        self.0.borrow_mut().push(state);
    }

    pub(crate) fn truncate(&self, len: usize) {
        let mut states = self.0.borrow_mut();
        for state in states.drain(len..) {
            state.set(WindowState::Closed);
        }
    }

    pub(crate) fn close_all(&self) {
        self.truncate(0);
    }
}

impl_extractor!(PopupMenuStateGroup);

impl PickerMenuSlot {
    pub(crate) fn new() -> Self {
        Self {
            open: Rc::new(Cell::new(false)),
        }
    }
}

fn popup_enter_animation() -> Animation {
    Animation::bezier(Duration::from_millis(120), 0.2, 0.0, 0.0, 1.0)
}

fn animated_popup_panel(content: impl View, group: PopupMenuStateGroup) -> impl View {
    let opacity = Binding::f32(0.0);
    let scale = Binding::f32(0.96);
    let enter_animation = popup_enter_animation();
    content
        .opacity(opacity.clone().with_animation(enter_animation.clone()))
        .scale(
            scale.clone().with_animation(enter_animation.clone()),
            scale.clone().with_animation(enter_animation),
        )
        .on_appear(move || {
            opacity.set(0.96);
            scale.set(1.0);
        })
        .with(group)
}

pub(crate) fn popup_menu_size(
    nodes: &[PopupMenuNode],
    metrics: TextContextMenuMetrics,
) -> (f64, f64) {
    let max_label_chars = nodes
        .iter()
        .filter_map(|node| match node {
            PopupMenuNode::Command { plain_label, .. }
            | PopupMenuNode::Menu { plain_label, .. } => Some(plain_label.chars().count()),
            PopupMenuNode::Divider => None,
        })
        .max()
        .unwrap_or(0) as f64;
    let width = (metrics.horizontal_padding * 2.0 + max_label_chars * metrics.width_per_char)
        .clamp(metrics.min_width, metrics.max_width);
    let height = (nodes.len() as f64 * metrics.row_height).max(metrics.row_height);
    (width, height)
}

pub(crate) fn popup_menu_window(
    nodes: Vec<PopupMenuNode>,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
    depth: usize,
    metrics: TextContextMenuMetrics,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let (width, height) = popup_menu_size(&nodes, metrics);
    let popup_origin_x = origin.x;
    let popup_origin_y = origin.y;
    let group_for_content = group.clone();
    let state_for_content = state.clone();
    let nodes_for_content = nodes.clone();
    let popup_content = move || {
        let mut rows = Vec::with_capacity(nodes_for_content.len());
        for (index, node) in nodes_for_content.clone().into_iter().enumerate() {
            match node {
                PopupMenuNode::Command {
                    label,
                    action,
                    disabled,
                    ..
                } => {
                    let button = Button::new(label).style(ButtonStyle::Borderless).action(
                        move |group: PopupMenuStateGroup, env: Environment| {
                            if disabled {
                                return;
                            }
                            group.close_all();
                            call_action_discarding_result(&action, &env);
                        },
                    );
                    rows.push(AnyView::new(button));
                }
                PopupMenuNode::Divider => rows.push(AnyView::new(Divider)),
                PopupMenuNode::Menu { label, items, .. } => {
                    let next_depth = depth + 1;
                    let child_origin = LayoutPoint::new(
                        popup_origin_x + width as f32,
                        popup_origin_y + (metrics.row_height * index as f64) as f32,
                    );
                    let button = Button::new(label).style(ButtonStyle::Borderless).action(
                        move |group: PopupMenuStateGroup, env: Environment| {
                            if items.is_empty() {
                                return;
                            }
                            group.truncate(next_depth);
                            let (window, child_state) = popup_menu_window(
                                items.clone(),
                                child_origin,
                                group.clone(),
                                next_depth,
                                metrics,
                            );
                            group.push(child_state);
                            env.get::<WindowManager>()
                                .expect(
                                    "hydrolysis popup menus require WindowManager in environment",
                                )
                                .show(window);
                        },
                    );
                    rows.push(AnyView::new(button));
                }
            }
        }
        let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            rows.into_iter().collect();
        AnyView::new(animated_popup_panel(
            menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .background(
                    RoundedRectangle::new((metrics.corner_radius / metrics.min_width) as f32)
                        .fill(waterui::Color::new(Surface)),
                ),
            group_for_content.clone(),
        ))
    };
    let mut popup = Window::new(
        TEXT_CONTEXT_MENU_WINDOW_TITLE,
        state_for_content,
        popup_content,
    )
    .style(WindowStyle::Borderless)
    .resizable(false)
    .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

pub(crate) fn picker_menu_window(
    entries: Vec<PickerMenuEntry>,
    selection: Binding<Id>,
    open: Rc<Cell<bool>>,
    origin: LayoutPoint,
    width: f64,
    row_height: f64,
    group: PopupMenuStateGroup,
    selected: Id,
    corner_radius: f64,
) -> (Window, Binding<WindowState>) {
    assert!(
        !entries.is_empty(),
        "hydrolysis picker menu requires entries"
    );
    assert!(
        width.is_finite() && width > 0.0,
        "hydrolysis picker menu requires positive finite width"
    );
    assert!(
        row_height.is_finite() && row_height > 0.0,
        "hydrolysis picker menu requires positive finite row height"
    );
    let state = Binding::container(WindowState::Normal);
    let height = row_height * entries.len() as f64;
    let state_for_content = state.clone();
    let entries_for_content = entries.clone();
    let group_for_content = group.clone();
    let open_for_disappear = Rc::clone(&open);
    let popup_content = move || {
        let mut rows = Vec::with_capacity(entries_for_content.len());
        for entry in entries_for_content.clone() {
            let tag = entry.tag;
            let is_selected = tag == selected;
            let row_selection = selection.clone();
            let row_group = group_for_content.clone();
            let row_open = Rc::clone(&open_for_disappear);
            let button = Button::new(entry.label)
                .style(ButtonStyle::Borderless)
                .action(move || {
                    if row_selection.get() != tag {
                        row_selection.set(tag);
                    }
                    row_open.set(false);
                    row_group.close_all();
                });
            let row_background = if is_selected {
                Color::new(SurfaceVariant).with_opacity(0.72)
            } else {
                Color::transparent()
            };
            rows.push(AnyView::new(
                Frame::new(button)
                    .width(width as f32)
                    .height(row_height as f32)
                    .background(RoundedRectangle::new(0.02).fill(row_background)),
            ));
        }
        let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            rows.into_iter().collect();
        let panel = menu_content
            .alignment(HorizontalAlignment::Leading)
            .spacing(0.0)
            .background(
                RoundedRectangle::new((corner_radius / width) as f32)
                    .fill(Color::new(Surface).with_opacity(0.96)),
            )
            .on_disappear({
                let open = Rc::clone(&open_for_disappear);
                move || {
                    open.set(false);
                }
            });
        AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
    };
    let mut popup = Window::new("WaterUI Picker Menu", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

fn color_picker_palette() -> [(&'static str, Color); 12] {
    [
        ("Red", Color::srgb(0xba, 0x1a, 0x1a)),
        ("Orange", Color::srgb(0xc2, 0x41, 0x0c)),
        ("Amber", Color::srgb(0x8a, 0x5a, 0x00)),
        ("Green", Color::srgb(0x2e, 0x7d, 0x32)),
        ("Teal", Color::srgb(0x00, 0x79, 0x6b)),
        ("Cyan", Color::srgb(0x00, 0x6d, 0x8f)),
        ("Blue", Color::srgb(0x0b, 0x57, 0xd0)),
        ("Indigo", Color::srgb(0x44, 0x38, 0xca)),
        ("Purple", Color::srgb(0x7b, 0x1f, 0xa2)),
        ("Pink", Color::srgb(0xa0, 0x18, 0x55)),
        ("Brown", Color::srgb(0x79, 0x55, 0x48)),
        ("Grey", Color::srgb(0x5f, 0x63, 0x68)),
    ]
}

pub(crate) fn color_picker_window(
    value: Binding<Color>,
    support_alpha: bool,
    support_hdr: bool,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let width = 280.0;
    let swatch = 40.0;
    let gap = 8.0;
    let rows = 3.0;
    let alpha_row_height = if support_alpha { 48.0 } else { 0.0 };
    let hdr_row_height = if support_hdr { 48.0 } else { 0.0 };
    let height = 16.0 + rows * swatch + 2.0 * gap + alpha_row_height + hdr_row_height + 16.0;
    let group_for_content = group.clone();
    let state_for_content = state.clone();
    let popup_content = move || {
        let palette = color_picker_palette();
        let mut row_views = Vec::with_capacity(5);
        for row_index in 0..3 {
            let mut swatches = Vec::with_capacity(4);
            for column_index in 0..4 {
                let palette_index = row_index * 4 + column_index;
                let (label, color) = palette[palette_index].clone();
                let selected = value.clone();
                let group = group_for_content.clone();
                let swatch_color = color.clone();
                swatches.push(AnyView::new(
                    Frame::new(
                        Button::new(label)
                            .style(ButtonStyle::Borderless)
                            .action(move || {
                                selected.set(color.clone());
                                group.close_all();
                            })
                            .install(LabelDisplayMode::Hidden)
                            .background(RoundedRectangle::new(0.2).fill(swatch_color)),
                    )
                    .width(swatch as f32)
                    .height(swatch as f32),
                ));
            }
            let row: waterui_layout::stack::HStack<(Vec<AnyView>,)> =
                swatches.into_iter().collect();
            row_views.push(AnyView::new(row.spacing(gap)));
        }

        if support_alpha {
            let selected = value.clone();
            let group = group_for_content.clone();
            row_views.push(AnyView::new(
                Frame::new(
                    Button::new("50% opacity")
                        .style(ButtonStyle::Borderless)
                        .action(move || {
                            let current = selected.get();
                            selected.set(current.with_opacity(0.5));
                            group.close_all();
                        }),
                )
                .width((width - 32.0) as f32)
                .height(40.0),
            ));
        }

        if support_hdr {
            let selected = value.clone();
            let group = group_for_content.clone();
            row_views.push(AnyView::new(
                Frame::new(
                    Button::new("HDR headroom")
                        .style(ButtonStyle::Borderless)
                        .action(move || {
                            let current = selected.get();
                            selected.set(current.with_headroom(1.0));
                            group.close_all();
                        }),
                )
                .width((width - 32.0) as f32)
                .height(40.0),
            ));
        }

        let content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            row_views.into_iter().collect();
        let panel = content
            .alignment(HorizontalAlignment::Leading)
            .spacing(gap)
            .padding_with(EdgeInsets::all(16.0))
            .background(RoundedRectangle::new(0.05).fill(Color::new(Surface).with_opacity(0.96)));
        AnyView::new(animated_popup_panel(panel, group_for_content.clone()))
    };
    let mut popup = Window::new("WaterUI Color Picker", state_for_content, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .background(Color::transparent());
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

impl HydrolysisRenderer {
    pub(crate) fn dismiss_active_popup_menu(&mut self) {
        if let Some(group) = self.popup_menu.active_popup_menu_group.take() {
            group.close_all();
        }
        for slot in &self.popup_menu.picker_menu_slots {
            slot.open.set(false);
        }
    }

    pub(crate) fn topmost_context_menu_target_at_point(
        &self,
        point: vello::kurbo::Point,
    ) -> Option<ContextMenuTarget> {
        self.hit_test
            .context_menu_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(_, target)| target.clone())
    }

    pub(crate) fn show_popup_menu_nodes(
        &mut self,
        nodes: Vec<PopupMenuNode>,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        if nodes.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let metrics = widget_theme(env).text_context_menu_metrics();
        let (window, state) = popup_menu_window(nodes, origin, group.clone(), 0, metrics);
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis popup menus require WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn show_picker_menu(
        &mut self,
        entries: Vec<PickerMenuEntry>,
        selection: Binding<Id>,
        open: Rc<Cell<bool>>,
        origin: LayoutPoint,
        width: f64,
        row_height: f64,
        selected: Id,
        corner_radius: f64,
        env: &Environment,
    ) -> bool {
        if entries.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        open.set(true);
        let group = PopupMenuStateGroup::new();
        let (window, state) = picker_menu_window(
            entries,
            selection,
            Rc::clone(&open),
            origin,
            width,
            row_height,
            group.clone(),
            selected,
            corner_radius,
        );
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis picker menus require WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn show_color_picker(
        &mut self,
        value: Binding<Color>,
        support_alpha: bool,
        support_hdr: bool,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let (window, state) =
            color_picker_window(value, support_alpha, support_hdr, origin, group.clone());
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis color picker requires WindowManager in environment")
            .show(window);
        self.popup_menu.active_popup_menu_group = Some(group);
        true
    }

    pub(crate) fn register_context_menu_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        items: nami::Computed<Vec<ResolvedMenuItem>>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.context_menu_targets.push(ContextMenuTarget {
            bounds,
            depth: self.render_depth,
            order,
            items,
        });
    }
}
