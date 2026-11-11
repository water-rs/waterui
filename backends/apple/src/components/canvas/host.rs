//! The leaf that hosts a canvas: a platform view whose layer holds the
//! lowered layer tree, and the frame loop that keeps the tree in step with
//! the canvas.
//!
//! One pass, [`Host::frame`], follows the box and the clock, lets the
//! content bring its installed recording up to date in place (or records it
//! when it has none), samples running animations, and applies the one
//! change the recording then reports to the tree. A change that needs no
//! animation runs one pass on the main queue; a running animation holds a
//! display-link clock until the canvas stops asking for frames.

use core::cell::{Cell, OnceCell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;

use cocoa_ui::display_link::FrameClock;
use cocoa_ui::main_queue::{enqueue, enqueue_local};
use cocoa_ui::mesh::MeshPass;
use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::{MainThreadMarker, PlatformView};
use kurbo::Size as KurboSize;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, StretchAxis, SubView, ViewDimensions,
};
use waterui_graphics::canvas::CanvasView;
use waterui_graphics::draw::{Content, ContentChange, Instant, LayoutSize, LiveOwner, SampleFlag};
use waterui_graphics::resources::{HeldResources, SceneResources};
use waterui_graphics::scene_view::{SceneInvalidator, resolve_scene_proposal, scene_stretch_axis};
use waterui_graphics::{SceneFrame, SceneUpdate};

use super::layers::{Ctx, Wake};
use super::resources::{Backend, Registry};
use super::text::CoreText;
use super::tree::Tree;
use crate::contract::NativeLeaf;
use crate::main_queue_owned::shared;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The bytes of image data a canvas may keep registered: what a layer
/// compositor holds on the GPU beside the app's own.
const IMAGE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// What a canvas keeps while mounted.
pub struct Host {
    mtm: MainThreadMarker,
    view: Retained<HostView>,
    scene: Rc<RefCell<CanvasView>>,
    registry: Rc<Registry>,
    resources: SceneResources,
    layout: LayoutSize,
    /// The registrations the installed recording names.
    held: RefCell<HeldResources>,
    content: RefCell<Option<Content>>,
    tree: RefCell<Option<Tree>>,
    /// The device scale the tree was built for.
    scale: Cell<f64>,
    /// Set by animated operands: a frame must sample.
    sample: SampleFlag,
    /// A pass is queued on the main queue.
    queued: Cell<bool>,
    /// The mesh gradient pass, opened by the first mesh.
    mesh: OnceCell<Rc<MeshPass>>,
    /// Shows finished mesh paints, from the GPU's completion thread.
    wake: Wake,
    clock: FrameClock,
    /// The intrinsic size layout was last told about.
    published: Cell<Option<waterui_core::layout::Size>>,
    /// This host, for the callbacks it hands out.
    me: Weak<Self>,
}

impl core::fmt::Debug for Host {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Host")
            .field("tree", &self.tree.borrow())
            .finish_non_exhaustive()
    }
}

impl LiveOwner for Host {
    fn changed(&self) {
        self.request();
    }
}

impl Host {
    /// Queues one pass for the end of this turn of the main loop.
    fn request(&self) {
        if self.queued.replace(true) {
            return;
        }
        let weak = self.me.clone();
        enqueue_local(self.mtm, move |_| {
            if let Some(host) = weak.upgrade() {
                host.queued.set(false);
                host.frame();
            }
        });
    }

    const fn ctx(&self) -> Ctx<'_> {
        Ctx {
            registry: &self.registry,
            scale: self.scale.get(),
            mesh: &self.mesh,
            wake: &self.wake,
        }
    }

    /// Shows the mesh paints the GPU has finished.
    fn show_meshes(&self) {
        if let Some(tree) = self.tree.borrow().as_ref() {
            tree.show_completed();
        }
    }

    /// One pass.
    fn frame(&self) {
        let bounds = cocoa_ui::view::bounds(&self.view);
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            // The layout pass that gives the view a box runs this again.
            return;
        }
        let scale = cocoa_ui::view::backing_scale_factor(&self.view);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "view boxes are logical points well inside f32"
        )]
        let (width, height) = (bounds.size.width as f32, bounds.size.height as f32);
        self.layout.set(&LayoutSize::change(
            KurboSize::new(f64::from(width), f64::from(height)),
            None,
        ));
        let now = Instant::now();
        let installed = self.content.borrow().is_some();
        let update = if installed {
            self.scene.borrow_mut().update_scene(
                &self.resources,
                SceneFrame {
                    width,
                    height,
                    time: now,
                },
            )
        } else {
            SceneUpdate::Record
        };
        let again = match update {
            SceneUpdate::Retained { again } => again,
            SceneUpdate::Record => self.record(width, height),
        };
        if scale.to_bits() != self.scale.get().to_bits() {
            self.scale.set(scale);
            if let Some(tree) = self.tree.borrow_mut().take() {
                let rebuilt = Tree::new(tree.list(), self.ctx());
                tree.root().removeFromSuperlayer();
                self.install(rebuilt);
            }
        }
        let animating = {
            let mut content = self.content.borrow_mut();
            let content = content
                .as_mut()
                .expect("a pass has recorded or kept its content");
            let animating = content.sample(now).is_animating();
            if let Some(change) = content.take_change() {
                self.apply(change);
            }
            animating
        };
        self.drive(again || animating || self.sample.get());
        self.publish_attributes();
    }

    /// Records the content afresh; whether it asks for another frame.
    fn record(&self, width: f32, height: f32) -> bool {
        let mut names = self.resources.recording();
        let mut again = false;
        let mut content = Content::record(&self.layout, |recorder| {
            again = self
                .scene
                .borrow_mut()
                .build_scene(recorder, &mut names, width, height);
        });
        let owner: Weak<dyn LiveOwner> = self.me.clone();
        content.attach(owner, &self.sample);
        let previous = self.held.replace(names.finish());
        *self.content.borrow_mut() = Some(content);
        // The first change of a fresh content carries the whole list.
        if let Some(old) = self.tree.borrow_mut().take() {
            old.root().removeFromSuperlayer();
        }
        drop(previous);
        again
    }

    fn install(&self, tree: Tree) {
        let layer =
            cocoa_ui::layer::layer_of(&self.view).expect("a canvas host view is layer backed");
        layer.addSublayer(tree.root());
        *self.tree.borrow_mut() = Some(tree);
    }

    fn apply(&self, change: ContentChange) {
        let ctx = self.ctx();
        match change {
            ContentChange::Replace(picture) => {
                let tree = Tree::new(picture.display_list(), ctx);
                if let Some(old) = self.tree.borrow_mut().take() {
                    old.root().removeFromSuperlayer();
                }
                self.install(tree);
            }
            ContentChange::Update(updates) => {
                self.tree
                    .borrow_mut()
                    .as_mut()
                    .expect("an update follows the whole list")
                    .update(&updates, ctx);
            }
            ContentChange::Edit(edit) => {
                self.tree
                    .borrow_mut()
                    .as_mut()
                    .expect("an edit follows the whole list")
                    .edit(&edit, ctx);
            }
        }
    }

    /// Holds the display-link clock while frames are wanted.
    fn drive(&self, wanted: bool) {
        if wanted {
            if !self.clock.is_running() {
                self.clock.start(&self.view);
            }
        } else if self.clock.is_running() {
            self.clock.stop();
        }
    }

    /// Tells layout and accessibility what the content now says.
    fn publish_attributes(&self) {
        let scene = self.scene.borrow();
        let current = scene.intrinsic_size();
        if current != self.published.replace(current) {
            crate::invalidation::invalidate_layout_hierarchy(&self.view);
        }
        cocoa_ui::view::set_accessibility_content(
            &self.view,
            scene.accessibility_label().as_deref(),
            scene.accessibility_value().as_deref(),
        );
    }
}

/// The layout face: a canvas takes its intrinsic size, else what it is
/// given.
struct CanvasSubView {
    host: Rc<Host>,
}

impl core::fmt::Debug for CanvasSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CanvasSubView").finish_non_exhaustive()
    }
}

impl SubView for CanvasSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let resolved = resolve_scene_proposal(self.host.scene.borrow().intrinsic_size(), proposal);
        ViewDimensions::new(waterui_core::layout::Size::new(
            resolved.width.unwrap_or(0.0),
            resolved.height.unwrap_or(0.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        scene_stretch_axis(self.host.scene.borrow().intrinsic_size())
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Mounts `view`, a canvas, as a layer tree.
pub fn mount(view: CanvasView, ctx: &crate::contract::RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host_view = HostView::new(mtm, cocoa_ui::Rect::ZERO);
    let platform: &PlatformView = &host_view;
    cocoa_ui::layer::ensure_layer(platform);
    let registry = Rc::new(Registry::default());
    let backend = Rc::new(Backend::new(Rc::clone(&registry), IMAGE_BUDGET_BYTES));
    let scene = Rc::new(RefCell::new(view));
    let host = Rc::new_cyclic(|weak: &Weak<Host>| {
        let tick = weak.clone();
        Host {
            mtm,
            me: weak.clone(),
            view: host_view.clone(),
            published: Cell::new(scene.borrow().intrinsic_size()),
            scene: Rc::clone(&scene),
            registry: Rc::clone(&registry),
            resources: {
                let resources = SceneResources::new(backend);
                resources.set_text(Rc::new(CoreText::new(mtm, Rc::clone(&registry))));
                resources
            },
            layout: LayoutSize::new(),
            held: RefCell::new(HeldResources::empty()),
            content: RefCell::new(None),
            tree: RefCell::new(None),
            scale: Cell::new(1.0),
            sample: SampleFlag::new(),
            queued: Cell::new(false),
            mesh: OnceCell::new(),
            wake: {
                let owned = shared(weak.clone(), mtm);
                Arc::new(move || {
                    let owned = Arc::clone(&owned);
                    enqueue(move |mtm| {
                        if let Some(host) = owned.get(mtm).upgrade() {
                            host.show_meshes();
                        }
                    });
                })
            },
            clock: FrameClock::new(mtm, move || {
                if let Some(host) = tick.upgrade() {
                    host.frame();
                }
            }),
        }
    });
    let invalidator: SceneInvalidator = {
        let weak = Rc::downgrade(&host);
        Rc::new(move || {
            if let Some(host) = weak.upgrade() {
                host.request();
            }
        })
    };
    scene.borrow_mut().set_invalidator(Some(invalidator));

    // A pass runs whenever the view's box, window or scale may have moved.
    host_view.set_layout_handler({
        let weak = Rc::downgrade(&host);
        move |_| {
            if let Some(host) = weak.upgrade() {
                host.frame();
            }
        }
    });
    host_view.set_window_handler({
        let weak = Rc::downgrade(&host);
        move |_| {
            if let Some(host) = weak.upgrade() {
                host.frame();
            }
        }
    });
    host_view.set_backing_changed_handler({
        let weak = Rc::downgrade(&host);
        move |_| {
            if let Some(host) = weak.upgrade() {
                host.frame();
            }
        }
    });

    let mut leaf = NativeLeaf::new(
        platform,
        CanvasSubView {
            host: Rc::clone(&host),
        },
    );
    leaf.keep(host_view);
    leaf.keep(Unmount(host));
    leaf
}

/// Clears the content's invalidator and stops the clock when the leaf goes.
struct Unmount(Rc<Host>);

impl Drop for Unmount {
    fn drop(&mut self) {
        self.0.clock.stop();
        self.0.scene.borrow_mut().set_invalidator(None);
        self.0.view.clear_handlers();
    }
}
