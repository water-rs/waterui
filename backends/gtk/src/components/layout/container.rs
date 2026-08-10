//! GTK container component using native GTK layout.

use gtk4::Widget;
use gtk4::prelude::*;
use waterui_core::{Environment, Native};
use waterui_layout::{StretchAxis, container::FixedContainer};

use crate::component::GtkComponent;
use crate::components::fixed_container_widget::WuiFixedContainer;
use crate::renderer::GtkRenderer;
use crate::util::effective_stretch_axis;

impl GtkComponent for Native<FixedContainer> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let (layout, children) = self.into_inner().into_inner();

        let children_with_axes: Vec<(gtk4::Widget, StretchAxis)> = children
            .into_iter()
            .map(|view| {
                let axis = effective_stretch_axis(&view);
                let widget = renderer.render_any(view, env);
                (widget, axis)
            })
            .collect();

        WuiFixedContainer::new(layout, children_with_axes).upcast()
    }
}
