use super::*;
use crate::platform::NativeKey;

/// Backend-owned input adapter for one embedded browser surface.
pub(crate) trait BrowserInputHandler {
    fn set_focus(&self, focused: bool);
    fn set_modifiers(&self, modifiers: Modifiers);
    fn pointer_move(&self, position: vello::kurbo::Point);
    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point);
    fn scroll(
        &self,
        position: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        is_line_delta: bool,
    );
    fn key(&self, pressed: bool, key: &KeyCode, native: Option<NativeKey>);
    fn text_input(&self, text: &str);
    fn commit_text(&self, text: &str);
}

#[derive(Clone)]
pub(crate) struct BrowserInputTarget {
    pub(crate) local_bounds: vello::kurbo::Rect,
    pub(crate) inverse_transform: vello::kurbo::Affine,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) handler: Rc<dyn BrowserInputHandler>,
}

impl BrowserInputTarget {
    pub(crate) fn local_position(&self, point: vello::kurbo::Point) -> Option<vello::kurbo::Point> {
        let local = self.inverse_transform * point;
        self.local_bounds
            .contains(local)
            .then_some(vello::kurbo::Point::new(
                local.x - self.local_bounds.x0,
                local.y - self.local_bounds.y0,
            ))
    }
}

impl HydrolysisRenderer {
    #[cfg(any(
        hydrolysis_linux_wpe_webview,
        hydrolysis_cef_webview,
        feature = "chromium"
    ))]
    pub(crate) fn register_browser_input_target(
        &mut self,
        local_bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        handler: Rc<dyn BrowserInputHandler>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let determinant = transform.determinant();
        assert!(
            determinant.is_finite() && determinant.abs() > f64::EPSILON,
            "embedded browser input transform must be finite and invertible"
        );
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.browser_targets.push(BrowserInputTarget {
            local_bounds,
            inverse_transform: transform.inverse(),
            depth: self.render_depth,
            order,
            handler,
        });
    }

    fn topmost_browser_target_at(
        &self,
        point: vello::kurbo::Point,
    ) -> Option<(usize, vello::kurbo::Point)> {
        self.hit_test
            .browser_targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                target
                    .local_position(point)
                    .map(|position| (index, position))
            })
            .max_by(|(left, _), (right, _)| {
                let left_target = &self.hit_test.browser_targets[*left];
                let right_target = &self.hit_test.browser_targets[*right];
                Self::target_hit_priority(left_target.depth, left_target.order, *left).cmp(
                    &Self::target_hit_priority(right_target.depth, right_target.order, *right),
                )
            })
    }

    pub(super) fn browser_target_wins_at(
        &self,
        point: vello::kurbo::Point,
        pointer_priority: Option<(usize, usize, usize)>,
        text_priority: Option<(usize, usize, usize)>,
    ) -> Option<(BrowserInputTarget, vello::kurbo::Point)> {
        let (index, position) = self.topmost_browser_target_at(point)?;
        let target = &self.hit_test.browser_targets[index];
        let browser_priority = Self::target_hit_priority(target.depth, target.order, index);
        if pointer_priority.is_some_and(|priority| priority > browser_priority)
            || text_priority.is_some_and(|priority| priority > browser_priority)
        {
            return None;
        }
        Some((target.clone(), position))
    }

    pub(crate) fn handle_browser_pointer_move(&mut self, point: vello::kurbo::Point) -> bool {
        if let Some(target) = self.hit_test.active_browser_target.as_ref() {
            let position = target.local_position(point).unwrap_or_else(|| {
                let local = target.inverse_transform * point;
                vello::kurbo::Point::new(
                    local.x - target.local_bounds.x0,
                    local.y - target.local_bounds.y0,
                )
            });
            target.handler.pointer_move(position);
            return true;
        }
        let pointer_priority = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, target)| Self::target_hit_priority(target.depth, target.order, index))
            .max();
        let text_priority = self.topmost_text_input_index_at_point(point).map(|index| {
            let target = &self.text_editing.text_input_targets[index];
            Self::target_hit_priority(target.depth, target.order, index)
        });
        let Some((target, position)) =
            self.browser_target_wins_at(point, pointer_priority, text_priority)
        else {
            return false;
        };
        target.handler.pointer_move(position);
        true
    }

    pub(crate) fn handle_browser_scroll(
        &mut self,
        point: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        is_line_delta: bool,
    ) -> bool {
        let Some((index, position)) = self.topmost_browser_target_at(point) else {
            return false;
        };
        self.hit_test.browser_targets[index].handler.scroll(
            position,
            delta_x,
            delta_y,
            is_line_delta,
        );
        true
    }

    pub(crate) fn handle_browser_key(
        &mut self,
        pressed: bool,
        key: &KeyCode,
        native: Option<NativeKey>,
        modifiers: Modifiers,
    ) -> bool {
        let Some(handler) = self.hit_test.focused_browser.as_ref() else {
            return false;
        };
        handler.set_modifiers(modifiers);
        handler.key(pressed, key, native);
        true
    }

    pub(crate) fn handle_browser_text_input(&mut self, text: &str) -> bool {
        let Some(handler) = self.hit_test.focused_browser.as_ref() else {
            return false;
        };
        handler.text_input(text);
        true
    }

    pub(crate) fn handle_browser_commit_text(&mut self, text: &str) -> bool {
        let Some(handler) = self.hit_test.focused_browser.as_ref() else {
            return false;
        };
        handler.commit_text(text);
        true
    }

    #[must_use]
    pub(crate) fn browser_has_focus(&self) -> bool {
        self.hit_test.focused_browser.is_some()
    }

    pub(crate) fn update_browser_modifiers(&mut self, modifiers: Modifiers) {
        if let Some(handler) = self.hit_test.focused_browser.as_ref() {
            handler.set_modifiers(modifiers);
        }
    }
}
