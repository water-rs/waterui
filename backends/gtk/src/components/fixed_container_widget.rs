//! A GTK widget implementing WaterUI's `FixedContainer` layout contract.
//!
//! This is a `gtk4::Fixed` subclass that delegates measurement and placement
//! to WaterUI's Rust `Layout` engine, mirroring the Apple backend behavior.

use std::cell::RefCell;

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{Fixed, Widget, glib};
use waterui_core::layout::{Layout, ProposalSize, Rect, Size, StretchAxis, SubView};

use crate::layout::{GtkSubView, place_children, update_positions};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct WuiFixedContainer {
        pub layout: RefCell<Option<Box<dyn Layout>>>,
        pub children: RefCell<Vec<(Widget, StretchAxis)>>,
        pub last_rects: RefCell<Vec<Rect>>,
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

            let size = layout.size_that_fits(proposal, &refs);
            let w = size.width.max(0.0).round() as i32;
            let h = size.height.max(0.0).round() as i32;
            match orientation {
                gtk4::Orientation::Horizontal => (w, w, -1, -1),
                gtk4::Orientation::Vertical => (h, h, -1, -1),
                _ => panic!("WuiFixedContainer: unexpected orientation {orientation:?}"),
            }
        }

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
            let widgets: Vec<Widget> = children.iter().map(|(w, _)| w.clone()).collect();

            // First placement adds children; subsequent placements only move/resize.
            let mut last_rects = self.last_rects.borrow_mut();
            if last_rects.is_empty() {
                place_children(self.obj().upcast_ref::<Fixed>(), &rects, &widgets);
            } else {
                update_positions(self.obj().upcast_ref::<Fixed>(), &rects, &widgets);
            }
            *last_rects = rects;
        }
    }
}

glib::wrapper! {
    pub struct WuiFixedContainer(ObjectSubclass<imp::WuiFixedContainer>)
        @extends Fixed, Widget;
}

impl WuiFixedContainer {
    #[must_use]
    pub fn new(layout: Box<dyn Layout>, children: Vec<(Widget, StretchAxis)>) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();

        *imp.layout.borrow_mut() = Some(layout);
        *imp.children.borrow_mut() = children;

        // Add children to the fixed container once; positioning is handled in allocate.
        for (child, _) in imp.children.borrow().iter() {
            obj.put(child, 0.0, 0.0);
        }

        obj
    }
}
