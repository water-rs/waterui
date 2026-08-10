//! A GTK widget implementing `WaterUI`'s `FixedContainer` layout contract.
//!
//! This is a `gtk4::Fixed` subclass that delegates measurement and placement
//! to `WaterUI`'s Rust `Layout` engine, mirroring the Apple backend behavior.

use std::cell::RefCell;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{Fixed, Widget, glib};
use waterui_core::layout::{
    Layout, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions, measure_layout,
};

use crate::layout::{GtkSubView, place_children, update_positions};

fn layout_debug_enabled() -> bool {
    std::env::var_os("WATERUI_GTK_LAYOUT_DEBUG").is_some()
}

fn trace_layout_rects(
    operation: &str,
    width: i32,
    height: i32,
    child_count: usize,
    rects: &[Rect],
) {
    tracing::debug!(
        target: "waterui::gtk::layout",
        operation,
        width,
        height,
        child_count,
        rect_count = rects.len(),
        "Laid out GTK fixed container"
    );
    for (index, rect) in rects.iter().enumerate().take(8) {
        tracing::debug!(
            target: "waterui::gtk::layout",
            operation,
            index,
            x = rect.x(),
            y = rect.y(),
            width = rect.width(),
            height = rect.height(),
            "GTK fixed-container child rectangle"
        );
    }
}

mod imp {
    use super::{
        Cast, CellRendererExt, CellRendererTextExt, Fixed, FixedImpl, GtkSubView, IsAttribute,
        IsRenderNode, Layout, ListModelExtManual, ObjectExt, ObjectImpl, ObjectInterfaceType,
        ObjectSubclass, ObjectSubclassExt, ObjectSubclassType, PixbufAnimationExt,
        PixbufAnimationExtManual, ProposalSize, RecentManagerExt, Rect, RefCell, RendererExt,
        ScaleExt, Size, SocketClientExt, SocketControlMessageExt, SocketControlMessageImpl,
        SocketExt, StretchAxis, SubView, SurfaceExt, TextTagExt, TextureExt, TreeModelExt,
        TypeModuleExt, Widget, WidgetExt, WidgetImpl, WidgetImplExt, glib, layout_debug_enabled,
        measure_layout, place_children, trace_layout_rects, update_positions,
    };

    #[derive(Debug, Default)]
    pub struct WuiFixedContainer {
        pub layout: RefCell<Option<Box<dyn Layout>>>,
        pub children: RefCell<Vec<(Widget, StretchAxis)>>,
        pub last_rects: RefCell<Vec<Rect>>,
        pub last_size: RefCell<Option<(i32, i32)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WuiFixedContainer {
        const NAME: &'static str = "WuiFixedContainer";
        type Type = super::WuiFixedContainer;
        type ParentType = Fixed;
    }

    impl ObjectImpl for WuiFixedContainer {}

    impl FixedImpl for WuiFixedContainer {}

    impl WidgetImpl for WuiFixedContainer {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_precision_loss,
            reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
        )]
        fn measure(&self, orientation: gtk4::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let layout_borrow = self.layout.borrow();
            let Some(layout) = layout_borrow.as_ref() else {
                panic!("WuiFixedContainer: missing layout (internal error)");
            };

            let children = self.children.borrow();
            if children.is_empty() {
                return (0, 0, -1, -1);
            }

            let subviews: Vec<GtkSubView> = children
                .iter()
                .map(|(w, axis)| GtkSubView::new(w.clone(), *axis))
                .collect();

            let refs: Vec<&dyn SubView> = subviews.iter().map(|v| v as &dyn SubView).collect();

            let proposal = if for_size >= 0 {
                let v = for_size as f32;
                match orientation {
                    gtk4::Orientation::Horizontal => ProposalSize::new(None, Some(v)),
                    gtk4::Orientation::Vertical => ProposalSize::new(Some(v), None),
                    _ => ProposalSize::UNSPECIFIED,
                }
            } else {
                ProposalSize::UNSPECIFIED
            };

            let size = measure_layout(layout.as_ref(), proposal, &refs).size;
            let w = size.width.max(0.0).round() as i32;
            let h = size.height.max(0.0).round() as i32;
            if layout_debug_enabled() {
                tracing::debug!(
                    target: "waterui::gtk::layout",
                    orientation = ?orientation,
                    for_size,
                    proposal_width = ?proposal.width,
                    proposal_height = ?proposal.height,
                    width = w,
                    height = h,
                    child_count = refs.len(),
                    "Measured GTK fixed container"
                );
            }
            match orientation {
                gtk4::Orientation::Horizontal => (w, w, -1, -1),
                gtk4::Orientation::Vertical => (h, h, -1, -1),
                _ => panic!("WuiFixedContainer: unexpected orientation {orientation:?}"),
            }
        }

        #[allow(
            clippy::cast_precision_loss,
            reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
        )]
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);

            let layout_borrow = self.layout.borrow();
            let Some(layout) = layout_borrow.as_ref() else {
                panic!("WuiFixedContainer: missing layout (internal error)");
            };

            let children = self.children.borrow();
            if children.is_empty() {
                return;
            }

            let subviews: Vec<GtkSubView> = children
                .iter()
                .map(|(w, axis)| GtkSubView::new(w.clone(), *axis))
                .collect();
            let refs: Vec<&dyn SubView> = subviews.iter().map(|v| v as &dyn SubView).collect();

            let bounds = Rect::from_size(Size {
                width: (width.max(0)) as f32,
                height: (height.max(0)) as f32,
            });

            // Measure first with bounds-based proposal so children know available width/height.
            let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
            let _ = layout.size_that_fits(proposal, &refs);

            let rects = layout.place(bounds, &refs);
            if layout_debug_enabled() {
                trace_layout_rects("allocate", width, height, children.len(), &rects);
            }

            // First placement adds children; subsequent placements only move/resize.
            let mut last_rects = self.last_rects.borrow_mut();
            if last_rects.is_empty() {
                place_children(self.obj().upcast_ref::<Fixed>(), &rects, &children);
            } else {
                update_positions(self.obj().upcast_ref::<Fixed>(), &rects, &children);
            }
            *last_rects = rects;
        }
    }
}

glib::wrapper! {
    pub struct WuiFixedContainer(ObjectSubclass<imp::WuiFixedContainer>)
        @extends Fixed, Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl WuiFixedContainer {
    pub(crate) fn layout_measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let imp = self.imp();
        let layout_borrow = imp.layout.borrow();
        let Some(layout) = layout_borrow.as_ref() else {
            panic!("WuiFixedContainer: missing layout (internal error)");
        };

        let children = imp.children.borrow();
        if children.is_empty() {
            return ViewDimensions::new(Size::zero());
        }

        let subviews: Vec<GtkSubView> = children
            .iter()
            .map(|(w, axis)| GtkSubView::new(w.clone(), *axis))
            .collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|v| v as &dyn SubView).collect();

        measure_layout(layout.as_ref(), proposal, &refs)
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "GTK widget geometry is integer pixels while WaterUI layout is f32"
    )]
    fn relayout(&self) {
        let width = self.width();
        let height = self.height();
        if width <= 0 || height <= 0 {
            return;
        }

        let imp = self.imp();
        {
            let mut last_size = imp.last_size.borrow_mut();
            if *last_size == Some((width, height)) {
                return;
            }
            *last_size = Some((width, height));
        }

        let layout_borrow = imp.layout.borrow();
        let Some(layout) = layout_borrow.as_ref() else {
            panic!("WuiFixedContainer: missing layout (internal error)");
        };

        let children = imp.children.borrow();
        if children.is_empty() {
            return;
        }

        let subviews: Vec<GtkSubView> = children
            .iter()
            .map(|(w, axis)| GtkSubView::new(w.clone(), *axis))
            .collect();
        let refs: Vec<&dyn SubView> = subviews.iter().map(|v| v as &dyn SubView).collect();

        let bounds = Rect::from_size(Size {
            width: (width.max(0)) as f32,
            height: (height.max(0)) as f32,
        });

        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let _ = layout.size_that_fits(proposal, &refs);
        let rects = layout.place(bounds, &refs);
        if layout_debug_enabled() {
            trace_layout_rects("relayout", width, height, children.len(), &rects);
        }

        let mut last_rects = imp.last_rects.borrow_mut();
        if last_rects.is_empty() {
            place_children(self.upcast_ref::<Fixed>(), &rects, &children);
        } else {
            update_positions(self.upcast_ref::<Fixed>(), &rects, &children);
        }
        *last_rects = rects;
    }

    /// Creates a container that lays `children` out with `layout`.
    #[must_use]
    pub fn new(layout: Box<dyn Layout>, children: Vec<(Widget, StretchAxis)>) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        if layout_debug_enabled() {
            tracing::debug!(
                target: "waterui::gtk::layout",
                widget_type = %obj.type_().name(),
                child_count = children.len(),
                "Created GTK fixed container"
            );
        }

        *imp.layout.borrow_mut() = Some(layout);
        *imp.children.borrow_mut() = children;

        // Add children to the fixed container once; positioning is handled in allocate.
        for (child, _) in imp.children.borrow().iter() {
            obj.put(child, 0.0, 0.0);
        }

        // In GTK4, subclassing Fixed does not provide reliable measure/allocate hooks for
        // custom layout logic. Reflow on each frame while mapped so nested containers pick
        // up parent allocation changes deterministically.
        obj.add_tick_callback(|widget, _| {
            widget.relayout();
            glib::ControlFlow::Continue
        });

        obj
    }
}
