//! GTK container component that uses `waterui-layout` for positioning.

use gtk4::prelude::*;
use gtk4::{Fixed, Widget};
use waterui_core::layout::{Layout, ProposalSize, Rect, SubView};
use waterui_core::{Environment, Native};
use waterui_layout::container::FixedContainer;

use crate::component::GtkComponent;
use crate::layout::{place_children, stretch_axis_for_widget, GtkSubView};
use crate::renderer::GtkRenderer;

impl GtkComponent for Native<FixedContainer> {
    /// Renders a `FixedContainer` to a GTK `Fixed` widget.
    ///
    /// This function:
    /// 1. Renders each child view to a GTK widget
    /// 2. Wraps each widget in a `GtkSubView` for measurement
    /// 3. Calls the layout's `size_that_fits` and `place` methods
    /// 4. Positions the widgets using `Fixed`
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let (layout, children) = self.into_inner().into_inner();

        // Render each child view to a GTK widget
        let child_widgets: Vec<Widget> = children
            .into_iter()
            .map(|view| renderer.render_any(view, env))
            .collect();

        // Create GtkSubView wrappers for measurement
        let subviews: Vec<GtkSubView> = child_widgets
            .iter()
            .map(|w| GtkSubView::new(w.clone(), stretch_axis_for_widget(w)))
            .collect();

        // Convert to trait object references for layout
        let subview_refs: Vec<&dyn SubView> = subviews.iter().map(|s| s as &dyn SubView).collect();

        // Create the Fixed container
        let fixed = Fixed::new();

        // Perform initial layout
        // TODO: Get actual available size from parent/window
        let proposal = ProposalSize {
            width: Some(800.0),
            height: Some(600.0),
        };

        let size = layout.size_that_fits(proposal, &subview_refs);
        let bounds = Rect::new(waterui_core::layout::Point::new(0.0, 0.0), size);
        let rects = layout.place(bounds, &subview_refs);

        // Place children
        place_children(&fixed, &rects, &child_widgets);

        // Set size request based on layout result
        fixed.set_size_request(size.width as i32, size.height as i32);

        fixed.upcast()
    }
}
