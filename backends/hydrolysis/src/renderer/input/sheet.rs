//! Modal sheet layer: `.sheet(...)` (water-rs/waterui#659).
//!
//! A sheet is window-modal while its `is_presented` binding reads `true`.
//! Every `.sheet` node reports its presentation to the window's [`SheetStack`]
//! each pass — the rendered flush and the semantic walk alike — and the stack
//! answers which sheet is on screen: one sheet per window, in presentation
//! order. A sheet presented while another is up waits behind it and comes up
//! when the one in front closes. A node that stops reporting left the tree, and
//! its sheet closes with it through the binding.
//!
//! After the tree flushes (and before `.dialog` alerts), [`HydrolysisRenderer::render_sheets`]
//! draws the front registration full-window — the `Sheet`'s modal layer: the
//! `Scrim` backdrop plus the detent-sized modal surface — above all regular
//! window content.
//!
//! Input and focus containment come from [`sheet_modal_environment`], which
//! extends the declaring node's environment with [`ModalInteraction`],
//! [`SheetLayerScope`], and `State(Dismiss)`.

use std::cell::RefCell;
use std::rc::Rc;

use kurbo::Affine;
use nami::{Binding, Computed, Signal as _, SignalExt as _};
use waterui_accessibility::{AccessibilityHidden, AccessibilityLabel, AccessibilityRole};
use waterui_backend_core::SlotView;
use waterui_controls::sheet::{SHEET_DEFAULT_LABEL_KEY, Sheet, SheetContentScope, SheetDetent};
use waterui_controls::theme::color::Scrim;
use waterui_core::env::With;
use waterui_core::extract::State as EnvState;
use waterui_core::handler::shared_action;
use waterui_core::layout::{
    Layout, LayoutError, PlacementInput, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement,
};
use waterui_core::plugin::Plugin;
use waterui_core::{Environment, Metadata, OneOf2, Str, View};
use waterui_graphics::color::Color;
use waterui_interaction::gesture::{GestureObserver, TapGesture};
use waterui_layout::container::FixedContainer;
use waterui_layout::frame::Frame;
use waterui_layout::padding::{EdgeInsets, Padding};
use waterui_layout::stack::{Alignment, HorizontalAlignment, vstack, zstack};
use waterui_shape::{CornerRadius, RoundedRectangle, ShapeExt as _};
use waterui_text::text::Text;

use super::InteractionKey;
use crate::renderer::{
    HeldChild, HydrolysisRenderer, RenderContext, RetainedSubview, SafeAreaLayout, bounded_proposal,
};
use crate::text::TextEngine;
use crate::theme::{ModalInteraction, SheetLook};

/// Marks the environment a presented sheet's modal layer renders in.
///
/// Backend-internal: the accessibility container that carries the sheet
/// surface's `Dialog` role reads it (when outside `SheetContentScope`) to
/// publish the node as a modal `Dialog`.
#[derive(Debug, Clone, Copy)]
pub struct SheetLayerScope;

impl Plugin for SheetLayerScope {}

/// One open sheet in the window's presentation order.
struct StackedSheet {
    marker: Rc<()>,
    is_presented: Binding<bool>,
    reported: bool,
}

/// The window's open sheets in presentation order. The first entry is the sheet
/// on screen; later ones wait for earlier entries to close.
#[derive(Default)]
pub struct SheetStack {
    entries: Vec<StackedSheet>,
    shown: Option<Rc<()>>,
    return_focus: Option<InteractionKey>,
}

/// What a closed pass asks of keyboard focus.
pub enum SheetFocus {
    /// The sheet on screen is unchanged: focus stays where it is.
    Keep,
    /// A sheet came on screen: focus moves into it.
    Enter,
    /// The last sheet closed: focus returns to where it was before the first
    /// one came up, or stays put when nothing held it then.
    Return(Option<InteractionKey>),
}

impl SheetStack {
    /// Opens a whole-tree pass: drops sheets closed since the last pass so the
    /// next sheet in line moves to the front before nodes report.
    pub(crate) fn begin_pass(&mut self) {
        self.entries.retain(|entry| entry.is_presented.snapshot());
        for entry in &mut self.entries {
            entry.reported = false;
        }
    }

    /// Reports a `.sheet` node for this pass and returns whether it is the
    /// front sheet on screen.
    pub(crate) fn report<S>(
        &mut self,
        marker: &Rc<()>,
        sheet: &Sheet<S>,
        presented: bool,
        focus: Option<InteractionKey>,
    ) -> bool {
        let position = self
            .entries
            .iter()
            .position(|entry| Rc::ptr_eq(&entry.marker, marker));
        match (presented, position) {
            (false, Some(index)) => {
                self.entries.remove(index);
                false
            }
            (false, None) => false,
            (true, Some(index)) => {
                self.entries[index].reported = true;
                index == 0
            }
            (true, None) => {
                if self.shown.is_none() && self.entries.is_empty() {
                    self.return_focus = focus;
                }
                self.entries.push(StackedSheet {
                    marker: Rc::clone(marker),
                    is_presented: sheet.is_presented().clone(),
                    reported: true,
                });
                self.entries.len() == 1
            }
        }
    }

    /// Closes a whole-tree pass: closes any sheet whose node left the tree and
    /// returns the keyboard-focus transition required.
    pub(crate) fn finish_pass(&mut self) -> SheetFocus {
        self.entries.retain(|entry| {
            if !entry.reported {
                entry.is_presented.set(false);
            }
            entry.reported
        });
        let front = self.entries.first().map(|entry| Rc::clone(&entry.marker));
        let changed = match (&front, &self.shown) {
            (Some(front), Some(shown)) => !Rc::ptr_eq(front, shown),
            (None, None) => false,
            _ => true,
        };
        let focus = match (changed, &front) {
            (false, _) => SheetFocus::Keep,
            (true, Some(_)) => SheetFocus::Enter,
            (true, None) => SheetFocus::Return(self.return_focus.take()),
        };
        self.shown = front;
        focus
    }
}

/// What a `.sheet(...)` node's flush registered for the post-flush pass.
pub struct RegisteredSheet<E: TextEngine> {
    pub(crate) env: Environment,
    pub(crate) build_layer: Rc<dyn Fn(SheetLook) -> RetainedSubview<E>>,
    pub(crate) content: Rc<RefCell<Option<RetainedSubview<E>>>>,
}

/// Builds Hydrolysis's self-drawn modal layer for `sheet` styled with `look`:
/// a full-window `Scrim` backdrop (tapping it invokes the user dismiss path)
/// and the detent-sized modal sheet surface carrying `AccessibilityRole::Dialog`.
pub fn sheet_modal_layer<E: TextEngine>(
    sheet: &Sheet<HeldChild<E>>,
    look: SheetLook,
) -> impl View + use<E> {
    let detent = sheet.primary_detent();
    let max_width = look.max_width;
    let scrim_sheet = sheet.clone();
    let scrim = Metadata::new(
        Metadata::new(
            Color::new(Scrim),
            GestureObserver::without_event(TapGesture::new(), move |env: Environment| {
                scrim_sheet.run_user_dismiss(&env);
            }),
        ),
        AccessibilityHidden::new(true),
    );
    FixedContainer::new(
        SheetLayout { detent, max_width },
        (
            scrim,
            SheetSurface {
                sheet: sheet.clone(),
                look,
            },
        ),
    )
}

/// Layout placing child 0 (scrim) across the full window and child 1 (sheet
/// surface) at the active [`SheetDetent`].
#[derive(Debug, Clone, Copy)]
struct SheetLayout {
    detent: SheetDetent,
    max_width: f32,
}

impl Layout for SheetLayout {
    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        )
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        let mut placements = Vec::with_capacity(children.len());
        if !children.is_empty() {
            let full_proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
            placements.push(SubviewPlacement::new(bounds, full_proposal));
        }
        if children.len() > 1 {
            let sheet_rect = match self.detent {
                SheetDetent::FullScreen => bounds,
                detent => {
                    let width = bounds.width().min(self.max_width);
                    let height = detent.resolve_height(bounds.height()).min(bounds.height());
                    let x = (bounds.width() - width).mul_add(0.5, bounds.x());
                    let y = bounds.y() + (bounds.height() - height);
                    Rect::new(Point::new(x, y), Size::new(width, height))
                }
            };
            let sheet_proposal =
                ProposalSize::new(Some(sheet_rect.width()), Some(sheet_rect.height()));
            placements.push(SubviewPlacement::new(sheet_rect, sheet_proposal));
        }
        placements
    }

    fn resolve_extent(
        &self,
        input: PlacementInput,
        _children: &[&dyn SubView],
    ) -> Result<Rect, LayoutError> {
        Ok(input.bounds)
    }
}

/// The sheet surface container drawn on Hydrolysis's modal layer: carries
/// `AccessibilityRole::Dialog`, resolves the accessible label, and wraps the
/// user content with [`SheetContentScope`].
struct SheetSurface<E: TextEngine> {
    sheet: Sheet<HeldChild<E>>,
    look: SheetLook,
}

impl<E: TextEngine> View for SheetSurface<E> {
    fn body(self, env: &Environment) -> impl View {
        let Self { sheet, look } = self;
        let spoken: Computed<Str> = sheet
            .accessible_label()
            .map_or_else(
                || Text::localized_or(SHEET_DEFAULT_LABEL_KEY, "Sheet").resolve(env),
                |label| label.resolve(env),
            )
            .content()
            .map(|styled| styled.to_plain())
            .computed();
        let content = With::new(SlotView::new(sheet.build_content()), SheetContentScope);
        let full_screen = sheet.primary_detent() == SheetDetent::FullScreen;
        if full_screen {
            let surface = Metadata::new(
                Metadata::new(
                    look.container_color,
                    GestureObserver::without_event(TapGesture::new(), || {}),
                ),
                AccessibilityHidden::new(true),
            );
            OneOf2::A(Metadata::new(
                Metadata::new(zstack((surface, content)), AccessibilityLabel::new(spoken)),
                AccessibilityRole::Dialog,
            ))
        } else {
            let body = if sheet.is_dismissible() && look.show_drag_handle {
                let handle = Metadata::new(
                    Padding::new(
                        EdgeInsets::new(
                            look.drag_handle_vertical_padding,
                            0.0,
                            look.drag_handle_vertical_padding,
                            0.0,
                        ),
                        Frame::new(
                            RoundedRectangle::new(CornerRadius::fraction(1.0))
                                .fill(look.drag_handle_color),
                        )
                        .width(look.drag_handle_width)
                        .height(look.drag_handle_height),
                    ),
                    AccessibilityHidden::new(true),
                );
                OneOf2::A(vstack((handle, content)).alignment(HorizontalAlignment::Center))
            } else {
                OneOf2::B(content)
            };
            let surface = Metadata::new(
                Metadata::new(
                    RoundedRectangle::new(CornerRadius::points(look.corner_radius))
                        .fill(look.container_color),
                    GestureObserver::without_event(TapGesture::new(), || {}),
                ),
                AccessibilityHidden::new(true),
            );
            OneOf2::B(Metadata::new(
                Metadata::new(
                    zstack((surface, body)).alignment(Alignment::Top),
                    AccessibilityLabel::new(spoken),
                ),
                AccessibilityRole::Dialog,
            ))
        }
    }
}

/// Builds the environment a sheet's modal layer presents in: the declaring
/// node's environment plus `ModalInteraction` (routing Escape and platform back
/// to [`Sheet::run_user_dismiss`]), [`SheetLayerScope`], and
/// `State(Dismiss)`.
pub fn sheet_modal_environment<S: 'static>(env: &Environment, sheet: &Sheet<S>) -> Environment {
    let escape = {
        let sheet = sheet.clone();
        shared_action(move |env: Environment| sheet.run_user_dismiss(&env))
    };
    env.extending(ModalInteraction::new(true, escape))
        .extending(SheetLayerScope)
        .extending(EnvState(sheet.dismiss_handle()))
}

impl<E: TextEngine> HydrolysisRenderer<E> {
    /// Draws the front `.sheet(...)` registration into the modal sheet layer.
    ///
    /// Runs after the main tree flush and before `.dialog` alerts so a
    /// confirmation alert opened from inside a sheet stacks cleanly above it.
    pub(crate) fn render_sheets(&mut self, transform: Affine, safe_area: &SafeAreaLayout) {
        assert!(
            self.material_group_scopes.is_empty(),
            "hydrolysis renderer: the sheet pass must start with an empty \
             material-scope stack — the tree flush leaves no scope open"
        );
        let Some(front) = self.popup_menu.front_sheet.take() else {
            return;
        };
        let window = self.window_bounds;
        let proposal = bounded_proposal(window);
        let theme = self.theme();
        let mut slot = front.content.borrow_mut();
        let content = slot.get_or_insert_with(|| (front.build_layer)(theme.sheet(&front.env)));
        let _ = content.patch_and_measure(self, &front.env, proposal);
        self.register_hit_test_occluder(window);
        content.place_detached(
            self,
            RenderContext::new(transform, window),
            &front.env,
            proposal,
            window,
            Some(safe_area.with_frame(window)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::SheetLayout;
    use waterui_controls::sheet::SheetDetent;
    use waterui_core::layout::{
        Layout, LayoutError, Point, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView,
        ViewDimensions,
    };

    struct DummySubView;

    impl SubView for DummySubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(0.0),
                proposal.height.unwrap_or(0.0),
            ))
        }

        fn resolve_extent(
            &self,
            bounds: Rect,
            proposal: ProposalSize,
        ) -> Result<ResolvedExtent, LayoutError> {
            ResolvedExtent::centred(bounds, self.measure(proposal).size)
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::Both
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn sheet_layout_respects_custom_max_width() {
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(1000.0, 800.0));
        let children: [&dyn SubView; 2] = [&DummySubView, &DummySubView];
        let placements = SheetLayout {
            detent: SheetDetent::Medium,
            max_width: 500.0,
        }
        .place(bounds, ProposalSize::new(1000.0, 800.0), &children);
        assert_eq!(placements.len(), 2);
        let surface = placements[1].frame;
        assert!((surface.width() - 500.0).abs() < f32::EPSILON);
        assert!((surface.x() - 250.0).abs() < f32::EPSILON);
        assert!((surface.height() - 400.0).abs() < f32::EPSILON);
        assert!((surface.y() - 400.0).abs() < f32::EPSILON);
    }
}
