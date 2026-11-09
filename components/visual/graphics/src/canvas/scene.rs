//! The scene leaf a canvas renders through: it replays the canvas's
//! retained recording into whichever target draws it, owning resource
//! registration and engine-generation recovery.

use alloc::rc::{Rc, Weak};
use alloc::string::{String, ToString as _};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::time::Duration;
use std::collections::HashMap;

use nami::watcher::{BoxWatcherGuard, Context, Metadata};
use nami::{Computed, Signal};
use waterui_core::animation::Animation;
use waterui_core::layout::{Affine2, Size};
use waterui_core::{Environment, Str};

use crate::animation::engine_animation;
use crate::canvas::Canvas;
use crate::canvas::cx::{Cx, Node, Recording, Shared};
use crate::canvas::drawing::Drawing;
use crate::canvas::geometry::lower_rect;
use crate::canvas::names::Names;
use crate::canvas::paint::{Paint, affine};
use crate::canvas::resource::{Font, ImageSource, ImageStatus, Pixels};
use crate::draw::{
    self, Draw as _, FontId, ImageId, Instant, Recorder, ResourceError, ShapeData, kurbo,
};
use crate::scene::resources::{RecordingResources, Registered};
use crate::scene_view::{SceneContent, SceneInvalidator};

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
/// recording it is bound into.
///
/// A change whose value names a resource that recording has no id for
/// cannot update in place: it asks the canvas to record again instead.
struct Gate<T: 'static, U> {
    source: Computed<T>,
    lower: fn(&T, &Names) -> Option<U>,
    names: Rc<RefCell<Names>>,
    shared: Weak<Shared>,
}

impl<T: 'static, U> Clone for Gate<T, U> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            lower: self.lower,
            names: Rc::clone(&self.names),
            shared: Weak::clone(&self.shared),
        }
    }
}

impl<T: 'static, U: 'static> Signal for Gate<T, U> {
    type Output = U;
    type Guard = BoxWatcherGuard;

    fn snapshot(&self) -> U {
        let value = self.source.snapshot();
        (self.lower)(&value, &self.names.borrow())
            .expect("a recorded operand's resources are registered before it is recorded")
    }

    fn watch(&self, watcher: impl Fn(Context<U>) + 'static) -> BoxWatcherGuard {
        let lower = self.lower;
        let names = Rc::clone(&self.names);
        let shared = Weak::clone(&self.shared);
        self.source.watch(move |change| {
            let (value, metadata) = change.into_parts();
            let lowered = lower(&value, &names.borrow());
            match lowered {
                Some(lowered) => watcher(Context::new(lowered, engine_metadata(metadata))),
                None => {
                    if let Some(shared) = shared.upgrade() {
                        shared.invalidate();
                    }
                }
            }
        })
    }
}

/// A registration and whether the recording being made still names it.
struct Held<I: crate::scene::resources::PlainId, S> {
    _source: S,
    registered: Result<Registered<I>, Arc<ResourceError>>,
    used: bool,
}

/// A drawing recorded for the current target.
struct Cached {
    _drawing: Drawing,
    picture: draw::Picture,
    used: bool,
}

/// What replaying the canvas into one recording needs.
struct Replay<'r, 'a> {
    resources: &'r mut RecordingResources<'a>,
    images: &'r mut HashMap<usize, Held<ImageId, ImageSource>>,
    fonts: &'r mut HashMap<usize, Held<FontId, Font>>,
    drawings: &'r mut HashMap<usize, Cached>,
    names: Rc<RefCell<Names>>,
    shared: Weak<Shared>,
    /// Statuses to publish once the replay is done: a status watcher may
    /// set a signal the canvas records from.
    statuses: Vec<(ImageSource, ImageStatus)>,
    /// Watchers of operands no recorded slot can carry: an image swap.
    guards: Vec<BoxWatcherGuard>,
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
            shared: Weak::clone(&self.shared),
        }
    }

    fn shape(&self, shape: &Computed<ShapeData>) -> Gate<ShapeData, ShapeData> {
        self.gate(shape, |shape, _| Some(shape.clone()))
    }

    fn transform(&self, transform: &Computed<Affine2>) -> Gate<Affine2, kurbo::Affine> {
        self.gate(transform, |transform, _| Some(affine(*transform)))
    }

    /// The id `image` draws by in this recording; `None` when the target
    /// refused it.
    fn image(&mut self, image: &ImageSource) -> Option<ImageId> {
        if let Some(id) = self.names.borrow().image(image) {
            return Some(id);
        }
        let held = self.images.entry(image.key()).or_insert_with(|| {
            let registered = match image.pixels() {
                Pixels::Rgba8(data) => self.resources.image(data.clone()),
                Pixels::Rgba16F(data) => self.resources.image16f(data.clone()),
            }
            .map_err(Arc::new);
            let status = match &registered {
                Ok(_) => ImageStatus::Ready,
                Err(error) => ImageStatus::Refused(Arc::clone(error)),
            };
            self.statuses.push((image.clone(), status));
            Held {
                _source: image.clone(),
                registered,
                used: false,
            }
        });
        held.used = true;
        let id = self.resources.name(held.registered.as_ref().ok()?);
        self.names.borrow_mut().insert_image(image, id);
        Some(id)
    }

    /// The id `font` draws by in this recording; `None` when the render
    /// thread is gone.
    ///
    /// # Panics
    ///
    /// When the target refuses the font: unreadable bytes, or a system
    /// family on a target with no platform font stack.
    fn font(&mut self, font: &Font) -> Option<FontId> {
        if let Some(id) = self.names.borrow().font(font) {
            return Some(id);
        }
        let held = self.fonts.entry(font.key()).or_insert_with(|| Held {
            _source: font.clone(),
            registered: self.resources.font(font.source()).map_err(Arc::new),
            used: false,
        });
        held.used = true;
        let registered = match &held.registered {
            Ok(registered) => registered,
            Err(error) if matches!(**error, ResourceError::Lost) => return None,
            Err(error) => panic!("a canvas glyph run's font was refused: {error}"),
        };
        let id = self.resources.name(registered);
        self.names.borrow_mut().insert_font(font, id);
        Some(id)
    }

    /// The paint operand as a live recorded paint; `None` when it samples
    /// an image the target refused.
    fn paint(&mut self, paint: &Computed<Paint>) -> Option<Gate<Paint, draw::Paint>> {
        if let Some(image) = paint.snapshot().image() {
            self.image(image)?;
        }
        Some(self.gate(paint, Paint::lower))
    }

    fn drawing(&mut self, drawing: &Drawing) -> Option<draw::Picture> {
        for image in drawing.images() {
            let _ = self.image(image);
        }
        for font in drawing.fonts() {
            self.font(font)?;
        }
        Some(picture(self.drawings, &self.names.borrow(), drawing))
    }
}

/// `drawing` recorded against `names`, from `cache` when it was recorded
/// for this target already.
fn picture(cache: &mut HashMap<usize, Cached>, names: &Names, drawing: &Drawing) -> draw::Picture {
    if let Some(cached) = cache.get_mut(&drawing.key()) {
        cached.used = true;
        return cached.picture.clone();
    }
    let recorded = drawing.lower(names, &mut |nested| picture(cache, names, nested));
    cache.insert(
        drawing.key(),
        Cached {
            _drawing: drawing.clone(),
            picture: recorded.clone(),
            used: true,
        },
    );
    recorded
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
                // A new image is a new registration: the canvas records
                // again rather than update a slot.
                let shared = Weak::clone(&cx.shared);
                let swapped = current.clone();
                cx.guards.push(image.watch(move |change| {
                    if *change.value() != swapped
                        && let Some(shared) = shared.upgrade()
                    {
                        shared.invalidate();
                    }
                }));
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
            Node::Scope(region) => replay(&region.borrow(), recorder, cx),
            Node::Each(list) => {
                list.borrow()
                    .for_each(&mut |item| replay(item, recorder, cx));
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

/// A mounted canvas: its retained recording, and the registrations the
/// target it is drawn by holds for it.
pub struct CanvasScene {
    shared: Rc<Shared>,
    root: Recording,
    images: HashMap<usize, Held<ImageId, ImageSource>>,
    fonts: HashMap<usize, Held<FontId, Font>>,
    drawings: HashMap<usize, Cached>,
    replay_guards: Vec<BoxWatcherGuard>,
    mounted: Option<Instant>,
    intrinsic: Computed<Option<Size>>,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
    _attribute_guards: [BoxWatcherGuard; 3],
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
        } = canvas;
        let shared = Shared::new(env.clone());
        let mut cx = Cx::new(Rc::clone(&shared));
        body(&mut cx);
        let root = cx.finish();
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
            images: HashMap::new(),
            fonts: HashMap::new(),
            drawings: HashMap::new(),
            replay_guards: Vec::new(),
            mounted: None,
            intrinsic,
            label,
            value,
            _attribute_guards: attribute_guards,
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
        let now = Instant::now();
        let mounted = *self.mounted.get_or_insert(now);
        if self.shared.clock.observed() {
            let elapsed: Duration = now.duration_since(mounted);
            self.shared.clock.set(&Context::from(elapsed));
        }
        let size = Size::new(width, height);
        if self.shared.size.snapshot() != size {
            self.shared.size.set(&Context::from(size));
        }

        for held in self.images.values_mut() {
            held.used = false;
        }
        for held in self.fonts.values_mut() {
            held.used = false;
        }
        for cached in self.drawings.values_mut() {
            cached.used = false;
        }
        let mut cx = Replay {
            resources,
            images: &mut self.images,
            fonts: &mut self.fonts,
            drawings: &mut self.drawings,
            names: Rc::default(),
            shared: Rc::downgrade(&self.shared),
            statuses: Vec::new(),
            guards: Vec::new(),
        };
        replay(&self.root, recorder, &mut cx);
        let Replay {
            statuses, guards, ..
        } = cx;
        self.replay_guards = guards;
        // What this recording no longer names is released once the host
        // installs it: the installed recording holds its own names.
        self.images.retain(|_, held| held.used);
        self.fonts.retain(|_, held| held.used);
        self.drawings.retain(|_, cached| cached.used);
        for (image, status) in statuses {
            image.set_status(&status);
        }
        self.shared.clock.observed()
    }

    fn rebuild_for_engine(&mut self) {
        // Registrations and recorded drawings belong to the replaced engine;
        // the next replay registers everything again on the new one.
        self.images.clear();
        self.fonts.clear();
        self.drawings.clear();
        self.replay_guards.clear();
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
