//! DOM planes: the system-compositor realization on wasm32 (#2334).
//!
//! The browser's compositor is the system compositor here, and the DOM is
//! its layer tree. A [`DomTarget`] hands the engine a host element; the
//! engine appends one stacking root to it and realizes the surface's stack
//! inside that root:
//!
//! - every engine part is a `<canvas>` with a WebGPU context of its own,
//!   filling the root, transparent above part 0 so the planes below show
//!   through;
//! - every hosted plane is the host's own element ([`HostedElement`], an
//!   `<iframe>` for a web view), placed by a CSS `matrix()` and clipped by a
//!   chain of SVG `<clipPath>`s, one per clipped level of its path.
//!
//! Stacking order is the `z-index` of each child of the root, never DOM
//! order: a browser reloads an `<iframe>` that leaves the document, so a
//! hosted element is appended to the root once and never moved again, and
//! a path whose shape changes only rewrites the element's style and its
//! clip chain. The root is an isolated stacking context, so the indices
//! order the surface's own children and nothing outside it.
//!
//! Only hosted content is promoted. An external frame has no buffer the
//! browser composites itself ([`Compositor::shows`]), and a recorded capture
//! would cost a canvas and a full-surface texture to save the engine one
//! composition ([`Compositor::CAPTURES`]), so both stay composited in the
//! engine.
//!
//! Engine canvases never take pointer events: input over engine content
//! reaches the host's element under the root, and a hosted element takes
//! its own. Which content occludes a hosted element for input is the
//! host's decision, as it is for an `NSView` on macOS.

use std::fmt::Write as _;

use kurbo::{Affine, Size};
use rustc_hash::FxHashMap;
use wasm_bindgen::JsCast as _;
use web_sys::{Element, HtmlCanvasElement, HtmlElement};

use cherenkov::{FillRule, LayerId, RenderError, ShapeData, SurfaceError};

use super::{
    Candidate, Composition, Compositor, Level, Placement, PlaneContent, Presentation, SystemPlanes,
};
use crate::interop::ExternalFrame;
use crate::render::present::{OutputRequest, WindowSurface};

/// The most hosted planes on one surface. Each one opens an engine part
/// above it — a canvas and a full-surface texture — so the budget bounds
/// memory as well as the number of browser compositing layers.
const BUDGET: usize = 2;

/// The flattening tolerance, in a clip's own units, for curves the SVG
/// path data carries as cubic segments.
const CLIP_TOLERANCE: f64 = 0.01;

/// The SVG namespace `<svg>`, `<clipPath>` and `<path>` are created in.
const SVG: &str = "http://www.w3.org/2000/svg";

/// A surface the browser composites: parts and hosted elements stacked
/// inside a stacking root the engine appends to `parent`.
///
/// `size` is the surface's device-pixel size. The root fills `parent`'s
/// content box, so every engine canvas is laid out at `parent`'s size and
/// its backing store is `size`: the host sizes `parent`, and keeps `size`
/// equal to that box times the device pixel ratio, as it does for a
/// single canvas.
#[derive(Debug)]
pub struct DomTarget {
    pub(crate) parent: HtmlElement,
    pub(crate) size: (u32, u32),
    pub(crate) transparent: bool,
    pub(crate) refresh: cherenkov::RefreshRange,
}

impl DomTarget {
    /// Presents inside `parent` at `size` device pixels.
    #[must_use]
    pub const fn new(parent: HtmlElement, size: (u32, u32)) -> Self {
        Self {
            parent,
            size,
            transparent: false,
            refresh: cherenkov::DEFAULT_REFRESH,
        }
    }

    /// Lets the page show through transparent pixels: the bottom part
    /// presents premultiplied instead of opaque.
    #[must_use]
    pub const fn transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }

    /// Sets the refresh range for backend animation and presentation
    /// retries.
    ///
    /// # Panics
    /// When the range is empty or includes zero.
    #[must_use]
    pub fn rate(mut self, rate: cherenkov::RefreshRange) -> Self {
        assert!(
            *rate.start() > 0 && !rate.is_empty(),
            "refresh range must be positive and ordered"
        );
        self.refresh = rate;
        self
    }

    /// The size the parts are allocated at.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.size
    }
}

impl From<DomTarget> for crate::GpuTarget {
    fn from(target: DomTarget) -> Self {
        Self::Dom(target)
    }
}

/// An element the host supplies — an `<iframe>` for a web view — shown on
/// a plane of its own (`cherenkov::HostedLayers`). The wasm32 hosted
/// object.
///
/// The engine appends the element to its stacking root on the first frame
/// that places it and removes it when its binding ends; in between it never
/// moves it — a frame that does not place it hides it in place — so an
/// `<iframe>` keeps its document. The engine owns the
/// element's `position`, `left`, `top`, `margin`, `box-sizing`, `width`,
/// `height`, `transform`, `transform-origin`, `z-index`, `opacity`, `clip-path`, `visibility` and
/// `pointer-events`; everything else is the host's.
#[derive(Clone)]
pub struct HostedElement {
    element: HtmlElement,
}

impl HostedElement {
    /// Wraps `element`.
    #[must_use]
    pub const fn new(element: HtmlElement) -> Self {
        Self { element }
    }

    /// Whether `self` and `other` hold the same element.
    #[must_use]
    pub fn is(&self, other: &Self) -> bool {
        self.element == other.element
    }
}

impl std::fmt::Debug for HostedElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("HostedElement")
            .field(&self.element.tag_name())
            .finish()
    }
}

/// One engine part: its canvas and the canvas's WebGPU swapchain.
struct PartCanvas {
    canvas: HtmlCanvasElement,
    surface: WindowSurface,
}

/// A hosted element the root holds, and what its style last showed, so a
/// frame that moves nothing writes nothing.
struct HostedNode {
    element: HostedElement,
    /// Its clip chain, outermost first.
    clips: Vec<Clip>,
    /// The placement, extent, stacking index and CSS scale last written;
    /// `None` while hidden.
    shown: Option<(Placement, Size, usize, (f64, f64))>,
    /// The composition that last placed it.
    epoch: u64,
}

/// The DOM realization of a surface's planes.
pub struct DomPlanes {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    /// The isolated stacking root the engine appended to the host's
    /// element.
    root: HtmlElement,
    /// The `<defs>` of the root's `<svg>`, which holds every clip chain.
    defs: Element,
    parts: Vec<PartCanvas>,
    hosted: FxHashMap<LayerId, HostedNode>,
    /// Counts compositions, to find the bound elements one does not place.
    epoch: u64,
    size: (u32, u32),
    /// The bottom part's request; parts above it always present
    /// premultiplied, so the planes below show through.
    request: OutputRequest,
    /// Distinguishes this surface's clip ids from every other surface's on
    /// the page, whichever module or renderer created it: element ids are
    /// global to the document.
    serial: String,
    next_clip: u64,
}

impl std::fmt::Debug for DomPlanes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DomPlanes")
            .field("size", &self.size)
            .field("parts", &self.parts.len())
            .field("hosted", &self.hosted.len())
            .finish_non_exhaustive()
    }
}

impl DomPlanes {
    /// Appends the stacking root to `target`'s element and realizes part 0.
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when the page cannot create the
    /// root or the browser cannot present a WebGPU canvas.
    pub fn new(
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        target: &DomTarget,
    ) -> Result<Self, SurfaceError> {
        let document = target
            .parent
            .owner_document()
            .ok_or_else(|| unsupported("the target element is not in a document"))?;
        let root: HtmlElement = document
            .create_element("div")
            .map_err(|_| unsupported("cannot create the stacking root"))?
            .unchecked_into();
        style(
            &root,
            &[
                ("position", "relative"),
                ("width", "100%"),
                ("height", "100%"),
                ("overflow", "hidden"),
                ("isolation", "isolate"),
                ("pointer-events", "none"),
            ],
        );
        let svg = document
            .create_element_ns(Some(SVG), "svg")
            .map_err(|_| unsupported("cannot create the clip root"))?;
        attributes(
            &svg,
            &[
                ("width", "0"),
                ("height", "0"),
                ("aria-hidden", "true"),
                ("style", "position:absolute"),
            ],
        );
        let defs = document
            .create_element_ns(Some(SVG), "defs")
            .map_err(|_| unsupported("cannot create the clip definitions"))?;
        append(&svg, &defs)?;
        append(&root, &svg)?;
        append(&target.parent, &root)?;
        let mut planes = Self {
            instance: instance.clone(),
            adapter: adapter.clone(),
            device: device.clone(),
            root,
            defs,
            parts: Vec::new(),
            hosted: FxHashMap::default(),
            epoch: 0,
            size: target.size,
            request: OutputRequest {
                transparent: target.transparent,
                color_space: None,
                sync: crate::DisplaySync::Synchronized,
            },
            serial: page_unique(),
            next_clip: 0,
        };
        planes.ensure_parts(1)?;
        Ok(planes)
    }

    /// Makes the root hold exactly `count` part canvases, creating the
    /// missing ones on top and removing the surplus.
    fn ensure_parts(&mut self, count: usize) -> Result<(), SurfaceError> {
        while self.parts.len() > count {
            let part = self.parts.pop().expect("a surplus part exists");
            part.canvas.remove();
        }
        while self.parts.len() < count {
            let index = self.parts.len();
            let canvas: HtmlCanvasElement = self
                .root
                .owner_document()
                .ok_or_else(|| unsupported("the stacking root left its document"))?
                .create_element("canvas")
                .map_err(|_| unsupported("cannot create a part canvas"))?
                .unchecked_into();
            style(
                &canvas,
                &[
                    ("position", "absolute"),
                    ("inset", "0"),
                    ("width", "100%"),
                    ("height", "100%"),
                    ("display", "block"),
                    ("pointer-events", "none"),
                    ("z-index", &stack_index(Slot::Part(index)).to_string()),
                ],
            );
            append(&self.root, &canvas)?;
            let request = OutputRequest {
                transparent: index > 0 || self.request.transparent,
                ..self.request
            };
            let surface = WindowSurface::from_canvas(
                &self.instance,
                &self.adapter,
                &self.device,
                canvas.clone(),
                self.size,
                request,
            )?;
            self.parts.push(PartCanvas { canvas, surface });
        }
        Ok(())
    }

    /// Device pixels to the root's CSS pixels, per axis: the canvases fill
    /// the root, so the scale is the root's used size — fractional, and
    /// untouched by any transform on its ancestors, which applies to the
    /// canvases and the hosted elements alike — over the surface's device
    /// size.
    fn css_scale(&self) -> Result<(f64, f64), RenderError> {
        let style = self
            .root
            .owner_document()
            .and_then(|document| document.default_view())
            .ok_or_else(|| RenderError::Render("the stacking root left its window".into()))?
            .get_computed_style(&self.root)
            .ok()
            .flatten()
            .ok_or_else(|| RenderError::Render("the stacking root has no computed style".into()))?;
        let used = |property: &str| {
            let value = style
                .get_property_value(property)
                .expect("a computed style reads a property");
            value
                .strip_suffix("px")
                .and_then(|number| number.parse::<f64>().ok())
                .ok_or_else(|| {
                    RenderError::Render(format!(
                        "the stacking root's used {property} is `{value}`, not a length in px"
                    ))
                })
        };
        Ok((
            used("width")? / f64::from(self.size.0),
            used("height")? / f64::from(self.size.1),
        ))
    }

    /// Shows the composition's hosted elements, each at its placement and
    /// stacking index, and hides every other bound element in place.
    fn host<'p>(
        &mut self,
        planes: impl Iterator<Item = (usize, &'p Placement, &'p HostedElement, Size)>,
    ) -> Result<(), RenderError> {
        // Reading the used size flushes the page's style, so a frame
        // without hosted planes does not.
        let mut scale = None;
        self.epoch += 1;
        for (index, placement, object, extent) in planes {
            let scale = match scale {
                Some(scale) => scale,
                None => *scale.insert(self.css_scale()?),
            };
            let node = match self.hosted.remove(&placement.layer) {
                Some(node) if node.element.is(object) => node,
                previous => {
                    if let Some(previous) = previous {
                        release(previous);
                    }
                    append(&self.root, &object.element).map_err(render)?;
                    HostedNode {
                        element: object.clone(),
                        clips: Vec::new(),
                        shown: None,
                        epoch: 0,
                    }
                }
            };
            let mut node = self.place(node, index, placement, extent, scale)?;
            node.epoch = self.epoch;
            self.hosted.insert(placement.layer, node);
        }
        for node in self.hosted.values_mut() {
            if node.epoch != self.epoch && node.shown.take().is_some() {
                style(
                    &node.element.element,
                    &[("visibility", "hidden"), ("pointer-events", "none")],
                );
            }
        }
        Ok(())
    }

    /// Writes `node`'s style and clip chain for `placement`, unless they
    /// already show it.
    fn place(
        &mut self,
        mut node: HostedNode,
        index: usize,
        placement: &Placement,
        extent: Size,
        scale: (f64, f64),
    ) -> Result<HostedNode, RenderError> {
        if node.shown.as_ref().is_some_and(|shown| {
            shown.0 == *placement && shown.1 == extent && shown.2 == index && shown.3 == scale
        }) {
            return Ok(node);
        }
        let device = placement.content_to_device();
        let css = Affine::scale_non_uniform(scale.0, scale.1) * device;
        // A singular placement maps the element onto a line or a point,
        // which the browser does not render: no clip space exists to
        // express, and none is needed.
        let clip = if device.determinant().is_normal() {
            self.clip_chain(&mut node.clips, &placement.path, device.inverse())?
        } else {
            for clip in node.clips.drain(..) {
                clip.clip_path.remove();
            }
            None
        };
        let element = &node.element.element;
        style(
            element,
            &[
                ("position", "absolute"),
                ("left", "0"),
                ("top", "0"),
                ("margin", "0"),
                ("box-sizing", "border-box"),
                ("transform-origin", "0 0"),
                ("width", &format!("{}px", extent.width)),
                ("height", &format!("{}px", extent.height)),
                ("transform", &css_matrix(css)),
                ("z-index", &stack_index(Slot::Plane(index)).to_string()),
                ("opacity", &placement.opacity.to_string()),
                ("visibility", "visible"),
                ("pointer-events", "auto"),
                (
                    "clip-path",
                    &clip.map_or_else(|| String::from("none"), |id| format!("url(#{id})")),
                ),
            ],
        );
        node.shown = Some((placement.clone(), extent, index, scale));
        Ok(node)
    }

    /// Writes the `<clipPath>` chain of `path`'s clipped levels in the
    /// element's own space — `element_from_device` maps device space there
    /// — each intersected with the one before through its own `clip-path`.
    /// The chain's elements are reused in place, so a scroll that moves the
    /// element under its clips only rewrites their shapes. Returns the
    /// innermost id, which clips by every level, or `None` when no level
    /// clips.
    fn clip_chain(
        &mut self,
        clips: &mut Vec<Clip>,
        path: &[Level],
        element_from_device: Affine,
    ) -> Result<Option<String>, RenderError> {
        let mut device_from_parent = Affine::IDENTITY;
        let mut count = 0;
        for level in path {
            let device_from_level = device_from_parent * level.transform;
            if let Some(shape) = &level.clip {
                if clips.len() == count {
                    let outer = clips.last().map(|clip: &Clip| clip.id.as_str());
                    let clip = self.new_clip(outer)?;
                    clips.push(clip);
                }
                attributes(
                    &clips[count].shape,
                    &[
                        ("d", &shape.to_path(CLIP_TOLERANCE).to_svg()),
                        ("clip-rule", clip_rule(shape)),
                        (
                            "transform",
                            &css_matrix(element_from_device * device_from_level),
                        ),
                    ],
                );
                count += 1;
            }
            device_from_parent = device_from_level * Affine::translate(-level.scroll);
        }
        for clip in clips.drain(count..) {
            clip.clip_path.remove();
        }
        Ok(clips.last().map(|clip| clip.id.clone()))
    }

    /// A `<clipPath>` with one `<path>`, intersected with `outer` when
    /// given.
    fn new_clip(&mut self, outer: Option<&str>) -> Result<Clip, RenderError> {
        let document = self
            .root
            .owner_document()
            .ok_or_else(|| RenderError::Render("the stacking root left its document".into()))?;
        let id = format!("cherenkov-clip-{}-{}", self.serial, self.next_clip);
        self.next_clip += 1;
        let clip_path = document
            .create_element_ns(Some(SVG), "clipPath")
            .map_err(|_| RenderError::Render("cannot create a clip path".into()))?;
        attributes(
            &clip_path,
            &[("id", &id), ("clipPathUnits", "userSpaceOnUse")],
        );
        if let Some(outer) = outer {
            attributes(&clip_path, &[("clip-path", &format!("url(#{outer})"))]);
        }
        let shape = document
            .create_element_ns(Some(SVG), "path")
            .map_err(|_| RenderError::Render("cannot create a clip shape".into()))?;
        append(&clip_path, &shape).map_err(render)?;
        append(&self.defs, &clip_path).map_err(render)?;
        Ok(Clip {
            id,
            clip_path,
            shape,
        })
    }
}

/// One `<clipPath>` of a hosted element's chain.
struct Clip {
    id: String,
    clip_path: Element,
    /// Its one `<path>`.
    shape: Element,
}

/// Where a child of the root stacks: part `n` below plane `n`, plane `n`
/// below part `n + 1` — the composition's own order, bottom first.
enum Slot {
    Part(usize),
    Plane(usize),
}

fn stack_index(slot: Slot) -> usize {
    match slot {
        Slot::Part(n) => 2 * n,
        Slot::Plane(n) => 2 * n + 1,
    }
}

/// A token no other surface on the page draws: 104 random bits in hex.
fn page_unique() -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "`Math.random` is in [0, 1); scaled by 2^52 it fits a u64 exactly"
    )]
    let draw = || (js_sys::Math::random() * 4_503_599_627_370_496.0) as u64;
    format!("{:013x}{:013x}", draw(), draw())
}

/// Detaches a hosted element and its clip chain: its binding ended, or
/// another element took its layer.
fn release(node: HostedNode) {
    for clip in node.clips {
        clip.clip_path.remove();
    }
    node.element.element.remove();
}

/// The CSS and SVG `matrix()` of `transform`.
fn css_matrix(transform: Affine) -> String {
    let [a, b, c, d, e, f] = transform.as_coeffs();
    let mut text = String::with_capacity(64);
    write!(text, "matrix({a},{b},{c},{d},{e},{f})").expect("writing to a String succeeds");
    text
}

/// The SVG fill rule a clip shape is filled with.
const fn clip_rule(shape: &ShapeData) -> &'static str {
    match shape {
        ShapeData::Path {
            rule: FillRule::EvenOdd,
            ..
        } => "evenodd",
        _ => "nonzero",
    }
}

fn style(element: &HtmlElement, properties: &[(&str, &str)]) {
    let declaration = element.style();
    for (property, value) in properties {
        declaration
            .set_property(property, value)
            .expect("a CSS declaration accepts a property write");
    }
}

fn attributes(element: &Element, values: &[(&str, &str)]) {
    for (name, value) in values {
        element
            .set_attribute(name, value)
            .expect("an element accepts an attribute write");
    }
}

fn append(parent: &Element, child: &Element) -> Result<(), SurfaceError> {
    parent
        .append_child(child)
        .map(drop)
        .map_err(|_| unsupported("the page refused an engine element"))
}

fn unsupported(cause: &str) -> SurfaceError {
    SurfaceError::UnsupportedTarget(format!("DOM planes: {cause}"))
}

fn render(error: SurfaceError) -> RenderError {
    RenderError::Render(error.to_string())
}

impl Compositor for DomPlanes {
    const BUDGET: usize = BUDGET;
    // CSS `opacity` fades an element and everything it shows.
    const HOSTS_OPACITY: bool = true;
    // A capture would cost a canvas and a full-surface part to save the
    // engine one composition.
    const CAPTURES: bool = false;

    // CSS `matrix()` carries any finite affine transform.
    fn expresses_transform(transform: Affine) -> bool {
        transform.as_coeffs().iter().all(|c| c.is_finite())
    }

    // An SVG `<clipPath>` carries any path with either fill rule, in any
    // space a `matrix()` reaches.
    fn expresses_clip(clip: &ShapeData) -> bool {
        clip.bounds().is_finite()
    }

    // No external frame is a buffer the browser composites itself.
    fn shows(_: &ExternalFrame) -> bool {
        false
    }
}

impl SystemPlanes for DomPlanes {
    // The DOM changes on the page's own thread inside `compose`; nothing
    // travels to another thread.
    type Commit = ();

    fn compose(&mut self, composition: Composition<'_>) -> Result<(Presentation, ()), RenderError> {
        self.ensure_parts(composition.parts.len()).map_err(render)?;
        self.host(composition.planes.iter().enumerate().map(
            |(index, plane)| match &plane.content {
                PlaneContent::Hosted { object, extent } => {
                    (index, plane.placement, *object, *extent)
                }
                PlaneContent::Frame { .. } | PlaneContent::Raster { .. } => {
                    unreachable!("the DOM realization is offered only hosted candidates")
                }
            },
        ))?;
        let mut presented = true;
        for (part, canvas) in composition.parts.iter().zip(&self.parts) {
            presented &= composition.presenter.present(
                composition.device,
                composition.queue,
                &canvas.surface,
                part.view,
                composition.display.headroom,
            )?;
        }
        Ok((
            if presented {
                Presentation::Presented
            } else {
                Presentation::Retry
            },
            (),
        ))
    }

    fn refresh<'a>(
        &mut self,
        frames: impl Iterator<Item = super::Plane<'a>>,
    ) -> Result<(), RenderError> {
        // A frame-only refresh is admitted only for promoted external
        // frames, and this realization promotes none.
        assert!(
            frames.count() == 0,
            "the DOM realization promotes no external frames"
        );
        Ok(())
    }

    // A hosted element leaves the page only when its binding ends: one a
    // composition does not place stays hidden in place, so an `<iframe>`
    // keeps its document while its layer is out of the tree.
    fn groom(&mut self, candidates: &FxHashMap<LayerId, Candidate>) {
        for (_, node) in self
            .hosted
            .extract_if(|layer, _| !candidates.contains_key(layer))
        {
            release(node);
        }
    }

    fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        for part in &mut self.parts {
            part.surface.resize(&self.device, size);
        }
    }

    fn reselect(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
    ) -> Result<(), SurfaceError> {
        for part in &mut self.parts {
            part.surface.reselect(adapter, device)?;
        }
        Ok(())
    }
}

impl Drop for DomPlanes {
    fn drop(&mut self) {
        for (_, node) in self.hosted.drain() {
            release(node);
        }
        self.root.remove();
    }
}
