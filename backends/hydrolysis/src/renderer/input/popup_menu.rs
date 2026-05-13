use super::*;

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

impl PopupMenuState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.picker_menu_cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.picker_menu_slots.truncate(self.picker_menu_cursor);
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
        AnyView::new(
            menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .with(group_for_content.clone()),
        )
    };
    let mut popup = Window::new(
        TEXT_CONTEXT_MENU_WINDOW_TITLE,
        state_for_content,
        popup_content,
    )
    .style(WindowStyle::Borderless)
    .resizable(false);
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
