//! The scene leaf a canvas renders through: it records the canvas once into
//! the recording its host installs, as one region per part, and edits that
//! recording in place when a part's structure changes — owning resource
//! registration and engine-generation recovery.

use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString as _};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use std::collections::HashMap;

use nami::watcher::{BoxWatcherGuard, Context, Metadata};
use nami::{Computed, Signal, SignalExt as _};
use waterui_core::animation::Animation;
use waterui_core::layout::{Affine2, Size};
use waterui_core::{Environment, Str};
use waterui_text::styled::StyledStr;

use crate::animation::engine_animation;
use crate::canvas::Canvas;
use crate::canvas::cx::{Cx, Node, Part, Recording, Shared, Stale};
use crate::canvas::drawing::{Drawing, WeakDrawing};
use crate::canvas::geometry::lower_rect;
use crate::canvas::names::Names;
use crate::canvas::paint::{Paint, affine};
use crate::canvas::resource::{Font, ImageSource, ImageStatus, Pixels};
use crate::canvas::size::Claim;
use crate::canvas::text::TextPlacement;
use crate::draw::{
    self, Draw as _, FontId, ImageId, Instant, Recorder, ResourceError, ShapeData, kurbo,
};
use crate::scene::resources::{HeldResources, RecordingResources, RegisteredText, SceneResources};
use crate::scene_view::{SceneContent, SceneFrame, SceneInvalidator, SceneUpdate};

/// A change's metadata as the recorder reads it: an app animation the
/// change was made under becomes the engine animation that animates the
/// operand.
fn engine_metadata(metadata: Metadata) -> Metadata {
    match (
        metadata.try_get::<draw::Animation>(),
        metadata.try_get::<Animation>(),
    ) {
        (None, Some(animation)) => metadata.with(engine_animation(&animation)),
        _ => metadata,
    }
}

/// A live operand lowered to its recorded form against the ids of the
/// part it is recorded in.
///
/// A change whose value names a resource that part has no id for cannot
/// update in place: it records the part again instead.
struct Gate<T: 'static, U> {
    source: Computed<T>,
    lower: fn(&T, &Names) -> Option<U>,
    names: Rc<RefCell<Names>>,
    shared: Weak<Shared>,
    part: Weak<Part>,
}

impl<T: 'static, U> Clone for Gate<T, U> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            lower: self.lower,
            names: Rc::clone(&self.names),
            shared: Weak::clone(&self.shared),
            part: Weak::clone(&self.part),
        }
    }
}

impl<T: 'static, U: 'static> Signal for Gate<T, U> {
    type Output = U;
    type Guard = BoxWatcherGuard;

    fn snapshot(&self) -> U {
        let value = self.source.snapshot();
        (self.lower)(&value, &self.names.borrow())
            .expect("a gate is read as its operand is recorded, right after `Replay::paint` registered the image its value names")
    }

    fn watch(&self, watcher: impl Fn(Context<U>) + 'static) -> BoxWatcherGuard {
        let lower = self.lower;
        let names = Rc::clone(&self.names);
        let shared = Weak::clone(&self.shared);
        let part = Weak::clone(&self.part);
        self.source.watch(move |change| {
            let (value, metadata) = change.into_parts();
            let lowered = lower(&value, &names.borrow());
            match lowered {
                Some(lowered) => watcher(Context::new(lowered, engine_metadata(metadata))),
                None => {
                    if let Some(shared) = shared.upgrade() {
                        shared.mark(Stale::Part(Weak::clone(&part)));
                    }
                }
            }
        })
    }
}

/// A drawing recorded for the current target, with the registrations its
/// recorded form names.
struct Cached {
    drawing: WeakDrawing,
    picture: draw::Picture,
    held: HeldResources,
}

/// What every part recorded in one host call shares.
struct Session<'s> {
    resources: &'s SceneResources,
    shared: Weak<Shared>,
    drawings: &'s RefCell<HashMap<usize, Cached>>,
    /// Statuses to publish once the call's recording is done: a status
    /// watcher may set a signal the canvas records from.
    statuses: RefCell<Vec<(ImageSource, ImageStatus)>>,
}

impl Session<'_> {
    /// Records `part`'s commands into `recorder`: the recorder of its
    /// region, which holds the registrations they name.
    fn record(&self, part: &Rc<Part>, recorder: &mut Recorder) {
        let mut cx = Replay {
            session: self,
            resources: self.resources.recording(),
            names: Rc::default(),
            part: Rc::downgrade(part),
        };
        replay(&part.recording.borrow(), recorder, &mut cx);
        let held = cx.resources.finish();
        recorder.keep(held);
    }

    /// `drawing` recorded for this target, from the cache when it was
    /// recorded already, and the registrations it names; `None` when the
    /// render thread is gone.
    fn drawing(&self, drawing: &Drawing) -> Option<(draw::Picture, HeldResources)> {
        if let Some(cached) = self.drawings.borrow().get(&drawing.key())
            && cached.drawing.is_alive()
        {
            return Some((cached.picture.clone(), cached.held.clone()));
        }
        let mut resources = self.resources.recording();
        let mut names = Names::default();
        for image in drawing.images() {
            let _ = register_image(&mut resources, &mut names, image, &self.statuses);
        }
        for font in drawing.fonts() {
            register_font(&mut resources, &mut names, font).ok()?;
        }
        let picture = drawing.lower(&names, &mut |nested| {
            // `None` only when the render thread went away since the fonts
            // above registered: the nested drawing is not drawn.
            let (picture, held) = self.drawing(nested)?;
            resources.hold(&held);
            Some(picture)
        });
        let held = resources.finish();
        self.drawings.borrow_mut().insert(
            drawing.key(),
            Cached {
                drawing: drawing.downgrade(),
                picture: picture.clone(),
                held: held.clone(),
            },
        );
        Some((picture, held))
    }

    fn publish_statuses(&self) {
        for (image, status) in self.statuses.take() {
            image.set_status(&status);
        }
    }
}

/// `drawing` recorded for the target `resources` belongs to, outside any
/// canvas: its recorded form and the registrations it names, which the
/// caller holds while the picture draws. Image statuses are published as
/// they are for a canvas. A font the target refuses is logged and its glyph
/// runs are not drawn.
///
/// # Panics
///
/// When the target's render thread is gone, which `resources`' owner, the
/// engine that renders the drawing, rules out while it lives.
#[cfg(any(feature = "gpu", feature = "cpu"))]
pub fn lower_drawing(
    drawing: &Drawing,
    resources: &SceneResources,
) -> (draw::Picture, HeldResources) {
    let drawings = RefCell::new(HashMap::new());
    let session = Session {
        resources,
        shared: Weak::new(),
        drawings: &drawings,
        statuses: RefCell::default(),
    };
    let lowered = session
        .drawing(drawing)
        .expect("the engine rendering a drawing outlives its recording");
    session.publish_statuses();
    lowered
}

/// Registers `image` and names it in one recording; `None` when the target
/// refused it, which its status reports.
fn register_image(
    resources: &mut RecordingResources<'_>,
    names: &mut Names,
    image: &ImageSource,
    statuses: &RefCell<Vec<(ImageSource, ImageStatus)>>,
) -> Option<ImageId> {
    if let Some(id) = names.image(image) {
        return Some(id);
    }
    let registered = match image.pixels() {
        Pixels::Rgba8(data) => resources.image(data.clone()),
        Pixels::Rgba16F(data) => resources.image16f(data.clone()),
    }
    .map_err(Arc::new);
    let status = match &registered {
        Ok(_) => ImageStatus::Ready,
        Err(error) => ImageStatus::Refused(Arc::clone(error)),
    };
    statuses.borrow_mut().push((image.clone(), status));
    let id = resources.name(&registered.ok()?);
    names.insert_image(image, id);
    Some(id)
}

/// The render thread is gone: nothing more can be registered on its target.
struct RenderThreadGone;

/// Registers `font` and names it in one recording; `Ok(None)` when the
/// target refuses the font — unreadable bytes, or a system family on a target
/// with no platform font stack — which is logged and leaves the glyph runs
/// that use it undrawn.
fn register_font(
    resources: &mut RecordingResources<'_>,
    names: &mut Names,
    font: &Font,
) -> Result<Option<FontId>, RenderThreadGone> {
    if let Some(id) = names.font(font) {
        return Ok(Some(id));
    }
    let registered = match resources.font(font.source()) {
        Ok(registered) => registered,
        Err(ResourceError::Lost) => return Err(RenderThreadGone),
        Err(error) => {
            tracing::error!(
                "a canvas glyph run's font was refused; its glyph runs are not drawn: {error}"
            );
            return Ok(None);
        }
    };
    let id = resources.name(&registered);
    names.insert_font(font, id);
    Ok(Some(id))
}

/// What replaying one part into the recorder of its region needs.
struct Replay<'r, 's> {
    session: &'r Session<'s>,
    resources: RecordingResources<'s>,
    names: Rc<RefCell<Names>>,
    part: Weak<Part>,
}

impl Replay<'_, '_> {
    fn gate<T: 'static, U: 'static>(
        &self,
        source: &Computed<T>,
        lower: fn(&T, &Names) -> Option<U>,
    ) -> Gate<T, U> {
        Gate {
            source: source.clone(),
            lower,
            names: Rc::clone(&self.names),
            shared: Weak::clone(&self.session.shared),
            part: Weak::clone(&self.part),
        }
    }

    fn shape(&self, shape: &Computed<ShapeData>) -> Gate<ShapeData, ShapeData> {
        self.gate(shape, |shape, _| Some(shape.clone()))
    }

    fn transform(&self, transform: &Computed<Affine2>) -> Gate<Affine2, kurbo::Affine> {
        self.gate(transform, |transform, _| Some(affine(*transform)))
    }

    fn image(&mut self, image: &ImageSource) -> Option<ImageId> {
        register_image(
            &mut self.resources,
            &mut self.names.borrow_mut(),
            image,
            &self.session.statuses,
        )
    }

    fn font(&mut self, font: &Font) -> Option<FontId> {
        register_font(&mut self.resources, &mut self.names.borrow_mut(), font)
            .ok()
            .flatten()
    }

    /// The paint operand as a live recorded paint; `None` when it samples
    /// an image the target refused.
    fn paint(&mut self, paint: &Computed<Paint>) -> Option<Gate<Paint, draw::Paint>> {
        if let Some(image) = paint.snapshot().image() {
            self.image(image)?;
        }
        Some(self.gate(paint, Paint::lower))
    }

    /// A watcher that records this part again when `changed` holds for a
    /// new value of the operand.
    fn restale_if<T: 'static>(
        &self,
        changed: impl Fn(&T) -> bool + 'static,
    ) -> impl Fn(Context<T>) + 'static {
        let shared = Weak::clone(&self.session.shared);
        let part = Weak::clone(&self.part);
        move |change| {
            if changed(change.value())
                && let Some(shared) = shared.upgrade()
            {
                shared.mark(Stale::Part(Weak::clone(&part)));
            }
        }
    }

    /// `text` laid out in `placement` by the target and registered, in the
    /// canvas's environment; `None` when the render thread is gone, the
    /// text has nothing to draw, or the target cannot lay the text out (it
    /// has no text stack, or a font of the text is refused), which is
    /// logged.
    fn text(&self, text: StyledStr, placement: TextPlacement) -> Option<RegisteredText> {
        let env = self.session.shared.upgrade()?.env.clone();
        match self.resources.text(text, placement, env) {
            Ok(registered) => registered,
            Err(ResourceError::Lost) => None,
            Err(error) => {
                tracing::error!("canvas text was refused; it is not drawn: {error}");
                None
            }
        }
    }

    /// Records a text command: the target's layout of the current text,
    /// placed by a live transform.
    ///
    /// A new text or a new way to lay it out is a new registration, so the
    /// part records again rather than update a slot. The watches live as
    /// long as this recording of the part; a moved origin alone follows the
    /// transform.
    fn record_text(
        &mut self,
        recorder: &mut Recorder,
        text: &Computed<StyledStr>,
        placement: &Computed<TextPlacement>,
    ) {
        let current = text.snapshot();
        let laid_out = placement.snapshot();
        // Every new text is a new layout.
        let text_guard: BoxWatcherGuard = text.watch(self.restale_if(|_: &StyledStr| true));
        let placement_guard: BoxWatcherGuard = placement.watch(
            self.restale_if(move |placement: &TextPlacement| !placement.same_layout(&laid_out)),
        );
        recorder.keep(text_guard);
        recorder.keep(placement_guard);
        let Some(registered) = self.text(current, laid_out) else {
            return;
        };
        let width = registered.size().width;
        let corner = placement
            .map(move |placement: TextPlacement| {
                let corner = placement.corner(width);
                Affine2::translate(corner.x, corner.y)
            })
            .computed();
        let layout = self.resources.name(registered.layout());
        recorder.text(layout, self.transform(&corner));
    }

    fn drawing(&mut self, drawing: &Drawing) -> Option<draw::Picture> {
        let (picture, held) = self.session.drawing(drawing)?;
        self.resources.hold(&held);
        Some(picture)
    }
}

fn replay(recording: &Recording, recorder: &mut Recorder, cx: &mut Replay<'_, '_>) {
    for node in &recording.nodes {
        match node {
            Node::Fill { shape, paint } => {
                if let Some(paint) = cx.paint(paint) {
                    recorder.fill(cx.shape(shape), paint);
                }
            }
            Node::Stroke {
                shape,
                style,
                paint,
            } => {
                if let Some(paint) = cx.paint(paint) {
                    let style = cx.gate(style, |style, _| Some(style.lower()));
                    recorder.stroke(cx.shape(shape), style, paint);
                }
            }
            Node::Shadow { shape, shadow } => {
                let shadow = cx.gate(shadow, |shadow, _| Some(shadow.lower()));
                recorder.shadow(cx.shape(shape), shadow);
            }
            Node::Image {
                image,
                dst,
                sampling,
            } => {
                let current = image.snapshot();
                // A new image is a new registration: the part records
                // again rather than update a slot. The watch lives as long
                // as this recording of the part.
                let shared = Weak::clone(&cx.session.shared);
                let part = Weak::clone(&cx.part);
                let swapped = current.clone();
                let guard: BoxWatcherGuard = image.watch(move |change| {
                    if *change.value() != swapped
                        && let Some(shared) = shared.upgrade()
                    {
                        shared.mark(Stale::Part(Weak::clone(&part)));
                    }
                });
                recorder.keep(guard);
                if let Some(id) = cx.image(&current) {
                    let dst = cx.gate(dst, |dst, _| Some(lower_rect(*dst)));
                    recorder.image(id, dst, sampling.lower());
                }
            }
            Node::Glyphs { run, paint } => {
                if cx.font(&run.snapshot().font).is_none() {
                    continue;
                }
                let Some(paint) = cx.paint(paint) else {
                    continue;
                };
                let run = cx.gate(run, |run, names| {
                    names.font(&run.font).map(|font| run.lower(font))
                });
                recorder.glyphs(run, paint);
            }
            Node::Text { text, placement } => cx.record_text(recorder, text, placement),
            Node::Draw { drawing, transform } => {
                if let Some(picture) = cx.drawing(drawing) {
                    recorder.picture(&picture, cx.transform(transform));
                }
            }
            Node::Clip { shape, body } => {
                let shape = cx.shape(shape);
                recorder.clip(shape, |recorder| replay(body, recorder, cx));
            }
            Node::Transform { transform, body } => {
                let transform = cx.transform(transform);
                recorder.transform(transform, |recorder| replay(body, recorder, cx));
            }
            Node::Layer { layer, body } => {
                let group = cx.gate(layer, |layer, _| Some(layer.lower()));
                recorder.group(group, |recorder| replay(body, recorder, cx));
            }
            Node::Part(part) => {
                let session = cx.session;
                part.install(recorder, &mut |part, recorder| {
                    session.record(part, recorder);
                });
            }
            Node::Each(list) => {
                let session = cx.session;
                list.borrow_mut().install(recorder, &mut |part, recorder| {
                    session.record(part, recorder);
                });
            }
        }
    }
}

/// Asks the host for another pass whenever `signal` changes.
fn invalidating<T: 'static>(signal: &Computed<T>, shared: &Rc<Shared>) -> BoxWatcherGuard {
    let shared = Rc::downgrade(shared);
    signal.watch(move |_| {
        if let Some(shared) = shared.upgrade() {
            shared.invalidate();
        }
    })
}

/// A mounted canvas: its retained recording, installed as one region per
/// part, and the drawings recorded for the target it is drawn by.
pub struct CanvasScene {
    shared: Rc<Shared>,
    root: Rc<Part>,
    drawings: RefCell<HashMap<usize, Cached>>,
    /// The first presented frame's instant: what [`Cx::time`] counts from.
    mounted: Option<Instant>,
    intrinsic: Computed<Option<Size>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
    _attribute_guards: [BoxWatcherGuard; 3],
    /// Holds the canvas's [`CanvasSize`] until the canvas unmounts.
    _size_claim: Claim,
}

impl CanvasScene {
    /// Runs the canvas's closure — once, for this mount — and keeps what it
    /// recorded.
    pub(crate) fn mount(canvas: Canvas, env: &Environment) -> Self {
        let Canvas {
            body,
            intrinsic,
            label,
            value,
            size,
        } = canvas;
        let size_claim = size.claim();
        let shared = Shared::new(env.clone(), size.cell());
        let mut cx = Cx::new(Rc::clone(&shared));
        body(&mut cx);
        let root = Part::new(cx.finish());
        // Layout and accessibility read these on the host's next pass,
        // which an invalidation schedules.
        let attribute_guards = [
            invalidating(&intrinsic, &shared),
            invalidating(&label, &shared),
            invalidating(&value, &shared),
        ];
        Self {
            shared,
            root,
            drawings: RefCell::new(HashMap::new()),
            mounted: None,
            intrinsic,
            label,
            value,
            _attribute_guards: attribute_guards,
            _size_claim: size_claim,
        }
    }

    /// Follows the box, and the frame clock while something observes it.
    fn follow(&mut self, width: f32, height: f32, time: Option<Instant>) {
        if let Some(time) = time {
            let mounted = *self.mounted.get_or_insert(time);
            self.shared
                .advance_clocks(time.saturating_duration_since(mounted));
        }
        let size = Size::new(width, height);
        if self.shared.size.snapshot() != size {
            self.shared.size.set(&Context::from(size));
        }
    }

    fn session<'s>(&'s self, resources: &'s SceneResources) -> Session<'s> {
        // A drawing nothing holds any more cannot be drawn again.
        self.drawings
            .borrow_mut()
            .retain(|_, cached| cached.drawing.is_alive());
        Session {
            resources,
            shared: Rc::downgrade(&self.shared),
            drawings: &self.drawings,
            statuses: RefCell::new(Vec::new()),
        }
    }
}

impl SceneContent for CanvasScene {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        self.follow(width, height, None);
        // A fresh recording installs every part anew.
        drop(self.shared.take_stale());
        let session = self.session(resources.scene_resources());
        self.root.install(recorder, &mut |part, recorder| {
            session.record(part, recorder);
        });
        session.publish_statuses();
        self.shared.clocks_running()
    }

    fn update_scene(&mut self, resources: &SceneResources, frame: SceneFrame) -> SceneUpdate {
        self.follow(frame.width, frame.height, Some(frame.time));
        let installed = self
            .root
            .handle
            .borrow()
            .as_ref()
            .is_some_and(draw::Region::is_recorded);
        if !installed {
            return SceneUpdate::Record;
        }
        let session = self.session(resources);
        // Each stale region records again in the order it changed; one
        // inside a region recorded again since is already current.
        for stale in self.shared.take_stale() {
            match stale {
                Stale::Part(part) => {
                    if let Some(part) = part.upgrade() {
                        part.rerecord(&mut |part, recorder| session.record(part, recorder));
                    }
                }
                Stale::Each(list) => {
                    if let Some(list) = list.upgrade() {
                        list.borrow_mut()
                            .apply(&mut |part, recorder| session.record(part, recorder));
                    }
                }
            }
        }
        session.publish_statuses();
        SceneUpdate::Retained {
            again: self.shared.clocks_running(),
        }
    }

    fn rebuild_for_engine(&mut self) {
        // Recorded drawings name the replaced engine's registrations, and
        // the installed recording is the replaced engine's; the next record
        // installs everything again on the new one.
        self.drawings.borrow_mut().clear();
        drop(self.root.handle.borrow_mut().take());
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.shared.set_invalidator(invalidator);
    }

    fn intrinsic_size(&self) -> Option<Size> {
        self.intrinsic.snapshot()
    }

    fn accessibility_label(&self) -> Option<String> {
        self.label.snapshot().map(|label| label.to_string())
    }

    fn accessibility_value(&self) -> Option<String> {
        self.value.snapshot().map(|value| value.to_string())
    }
}
