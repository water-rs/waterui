//! A GTK widget that clips its child to a `WaterUI` shape.
//!
//! Clipping happens in [`WidgetImpl::snapshot`], where the widget's allocated
//! size is finally known, so a normalized corner radius can be resolved against
//! the shorter side the way [`crate::shape_geometry`] describes. CSS cannot do
//! this: `border-radius` in percent resolves horizontally against the width and
//! vertically against the height, so a percentage that is round on a square
//! surface is elliptical on every other one.
//!
//! A custom path clips through `gtk_snapshot_push_fill`, the one GTK primitive
//! that clips to an arbitrary path; it arrived in GTK 4.14, which is why that
//! is the backend's floor.

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{Widget, glib, graphene, gsk};
use waterui_shape::{PathCommand, ShapeKind};

use super::gsk_path;
use crate::shape_geometry::{Corner, ShapeGeometry, resolve};
use crate::shape_path::bez_path;

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
        /// The unit-space outline, consulted only for a custom path: every
        /// other kind resolves through `shape_geometry`.
        pub commands: RefCell<Vec<PathCommand>>,
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

            let rect = match resolve(self.kind.get(), width, height) {
                ShapeGeometry::Rounded(rect) => rect,
                ShapeGeometry::CustomPath => {
                    let path = gsk_path(&bez_path(&self.commands.borrow(), width, height));
                    snapshot.push_fill(&path, gsk::FillRule::Winding);
                    self.parent_snapshot(snapshot);
                    snapshot.pop();
                    return;
                }
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
    /// Wraps `child` in a widget that clips it to `kind`, or to `commands`
    /// when the kind is [`ShapeKind::CustomPath`].
    #[must_use]
    pub fn new(kind: ShapeKind, commands: &[PathCommand], child: &Widget) -> Self {
        let widget: Self = glib::Object::new();
        widget.set_halign(gtk4::Align::Fill);
        widget.set_valign(gtk4::Align::Fill);
        widget.imp().kind.set(kind);
        *widget.imp().commands.borrow_mut() = commands.to_vec();
        child.set_parent(&widget);
        *widget.imp().child.borrow_mut() = Some(child.clone());
        widget
    }
}
