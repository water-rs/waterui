//! A GTK widget that clips its child to a `WaterUI` shape.
//!
//! Clipping happens in [`WidgetImpl::snapshot`], where the widget's allocated
//! size is finally known, so a normalized corner radius can be resolved against
//! the shorter side the way [`crate::shape_geometry`] describes. CSS cannot do
//! this: `border-radius` in percent resolves horizontally against the width and
//! vertically against the height, so a percentage that is round on a square
//! surface is elliptical on every other one.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{Widget, glib, graphene, gsk};
use waterui_shape::ShapeKind;

use crate::shape_geometry::{Corner, ShapeGeometry, resolve};

mod imp {
    // The glib subclass macros expand against the parent scope, so this module
    // deliberately re-exports it wholesale rather than tracking each generated use.
    #[allow(
        clippy::wildcard_imports,
        reason = "glib subclass macros expand against the parent scope"
    )]
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default)]
    pub struct WuiClipShape {
        pub kind: Cell<ShapeKind>,
        pub child: RefCell<Option<Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WuiClipShape {
        const NAME: &'static str = "WuiClipShape";
        type Type = super::WuiClipShape;
        type ParentType = Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk4::BinLayout>();
        }
    }

    impl ObjectImpl for WuiClipShape {
        fn dispose(&self) {
            if let Some(child) = self.child.borrow_mut().take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for WuiClipShape {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "GSK geometry is f32 while the resolved shape geometry is f64"
        )]
        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            let widget = self.obj();
            let width = f64::from(widget.width());
            let height = f64::from(widget.height());

            let ShapeGeometry::Rounded(rect) = resolve(self.kind.get(), width, height) else {
                unreachable!("WuiClipShape rejects a custom path in its constructor");
            };

            let bounds = graphene::Rect::new(
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            );
            if rect.is_rectangular() {
                snapshot.push_clip(&bounds);
            } else {
                let size = |corner: Corner| {
                    graphene::Size::new(corner.horizontal as f32, corner.vertical as f32)
                };
                let [top_left, top_right, bottom_right, bottom_left] = rect.corners;
                snapshot.push_rounded_clip(&gsk::RoundedRect::new(
                    bounds,
                    size(top_left),
                    size(top_right),
                    size(bottom_right),
                    size(bottom_left),
                ));
            }

            self.parent_snapshot(snapshot);
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// A single-child widget that clips its child to a [`ShapeKind`].
    pub struct WuiClipShape(ObjectSubclass<imp::WuiClipShape>)
        @extends Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl WuiClipShape {
    /// Wraps `child` in a widget that clips it to `kind`.
    ///
    /// # Panics
    ///
    /// Panics for [`ShapeKind::CustomPath`]. Clipping to an arbitrary path needs
    /// `gtk_snapshot_push_fill`, which arrived in GTK 4.14, and this backend
    /// targets 4.12. Clipping to a custom path has never worked on GTK; it used
    /// to panic out of the path recogniser instead of here.
    #[must_use]
    pub fn new(kind: ShapeKind, child: &Widget) -> Self {
        assert!(
            !matches!(kind, ShapeKind::CustomPath),
            "the GTK backend cannot clip to a custom path: gtk_snapshot_push_fill needs GTK 4.14 \
             and this backend targets 4.12"
        );

        let widget: Self = glib::Object::new();
        widget.set_halign(gtk4::Align::Fill);
        widget.set_valign(gtk4::Align::Fill);
        widget.imp().kind.set(kind);
        child.set_parent(&widget);
        *widget.imp().child.borrow_mut() = Some(child.clone());
        widget
    }
}
