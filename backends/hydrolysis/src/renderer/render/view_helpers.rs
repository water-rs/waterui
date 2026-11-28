// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use waterui_core::Computed;
use waterui_core::metadata::MetadataKey;

/// Defaults an environment scope gives every environment snapshot below it.
///
/// A `Metadata<Environment>` snapshot replaces the environment wholesale, so
/// a scope that sets a default for its whole subtree (a bar item's icon-only
/// labels) would stop at the first snapshot inside it. The build applies
/// these defaults to each snapshot it installs below the scope and carries
/// them on, so the subtree keeps one scope however its content was wrapped.
#[derive(Clone, Copy)]
pub struct SnapshotDefaults(pub fn(&Environment) -> Environment);

/// `snapshot` with the [`SnapshotDefaults`] `outer` carries applied and
/// carried on.
pub fn apply_snapshot_defaults(outer: &Environment, snapshot: Environment) -> Environment {
    let Some(&defaults) = outer.get::<SnapshotDefaults>() else {
        return snapshot;
    };
    let mut snapshot = (defaults.0)(&snapshot);
    snapshot.insert(defaults);
    snapshot
}

/// Extends `env` with an accessibility metadata value exactly like the retained
/// build's `Env`-scoping arm does. A composite body may snapshot its
/// environment into a `Metadata<Environment>` override (the `With`-style env
/// wrappers behind `.foreground()` and friends), so content resolves under the
/// same scoped environment the build-time `Env` node installs — otherwise the
/// snapshot, which replaces the environment wholesale when flattened, silently
/// drops the accessibility scoping above it.
pub fn a11y_scoped_env<T: MetadataKey + Clone + 'static>(
    env: &Environment,
    value: &T,
) -> Environment {
    let mut scoped = env.clone();
    scoped.insert(value.clone());
    scoped
}

/// The `Env` scoping for naming metadata (`.a11y_label()` / `.a11y_role()`).
///
/// Alongside the value, the scope carries a [`ScopedAccessibilitySemantics`]
/// identity so the node that ends up representing the wrapped view can claim it.
/// A control claims it by registering its own node; a composed container finds it
/// unclaimed and synthesizes the node naming itself. Nesting two of these (the
/// usual `.a11y_label(..).a11y_role(..)` pair) is harmless: only the innermost is
/// visible below, and it is the one every consumer resolves against.
#[cfg(feature = "accessibility")]
pub fn a11y_naming_scoped_env<T: MetadataKey + Clone + 'static, E: crate::text::TextEngine>(
    env: &Environment,
    value: &T,
) -> Environment {
    let mut scoped = a11y_scoped_env(env, value);
    scoped.insert(ScopedAccessibilitySemantics::<E>::new());
    scoped
}

/// Without an accessibility tree there is no naming scope to grow, so naming
/// metadata scopes the environment like every other value.
#[cfg(not(feature = "accessibility"))]
pub fn a11y_naming_scoped_env<T: MetadataKey + Clone + 'static, E: crate::text::TextEngine>(
    env: &Environment,
    value: &T,
) -> Environment {
    a11y_scoped_env(env, value)
}

/// Carries the accessibility naming scope across a `Metadata<Environment>`
/// override.
///
/// Naming metadata is *nearest-consumer*: the build deliberately leaves
/// `.a11y_label()` / `.a11y_role()` / `.a11y_id()` out of the environments
/// composite bodies snapshot, because a group label baked into one
/// would relabel every descendant leaf. The build-time `Env` node above is what
/// the emitting leaf reads instead — which works right up until the body
/// *itself* produces a snapshot, because flattening one replaces the
/// environment wholesale and takes that `Env` scope with it.
///
/// A view that wraps its content in a snapshot of the environment it was built
/// with does exactly that. Without this, `.a11y_label("City map")` above such a
/// view reached neither the content — `waterui-map-gpu` only generates its own
/// description when it finds no ambient label — nor the node hydrolysis
/// emitted, and the caller's name was silently replaced by a generated one.
///
/// Only what the snapshot does not already carry is restored, so a snapshot
/// taken *below* a naming scope still speaks for itself.
#[cfg(feature = "accessibility")]
pub fn restore_a11y_naming_scope<E: crate::text::TextEngine>(
    outer: &Environment,
    mut snapshot: Environment,
) -> Environment {
    macro_rules! restore {
        ($($ty:ty),+ $(,)?) => {
            $(
                if snapshot.get::<$ty>().is_none()
                    && let Some(value) = outer.get::<$ty>().cloned()
                {
                    snapshot.insert(value);
                }
            )+
        };
    }
    restore!(
        AccessibilityLabel,
        AccessibilityValue,
        AccessibilityRole,
        AccessibilityChildren,
        AccessibilityGeometry,
        ScopedAccessibilityIdentifier,
        // The claim identity travels with the name: carrying the label without
        // it would let the leaf render the name that a container above still
        // believes nobody spoke for, and both would emit it.
        ScopedAccessibilitySemantics<E>,
        // The same goes for text the claim consumes as its name: a snapshot
        // that drops the marker would let the leaf emit a `Label` the
        // container's name already repeats.
        AccessibilityNameFromContents,
    );
    snapshot
}

/// Without an accessibility tree there is no naming scope to carry.
#[cfg(not(feature = "accessibility"))]
pub const fn restore_a11y_naming_scope<E: crate::text::TextEngine>(
    _outer: &Environment,
    snapshot: Environment,
) -> Environment {
    snapshot
}

/// The `Env` scoping for a static [`AccessibilityState`]: a hidden state
/// suppresses the whole subtree's emission (a constant can never un-hide), and
/// the state is stored as a constant [`AccessibilityStateSignal`] so emission
/// resolves one code path for static and reactive state alike. Like the
/// reactive scope, it merges over the state an enclosing scope installed.
pub fn a11y_scoped_env_for_state(env: &Environment, value: &AccessibilityState) -> Environment {
    let mut scoped = env.clone();
    if value.is_hidden() {
        scoped.insert(AccessibilityHidden::new(true));
    }
    AccessibilityStateSignal::install(&mut scoped, Computed::constant(value.clone()));
    scoped
}

/// The `Env` scoping for a reactive [`AccessibilityStateSignal`]: the scope
/// merges field by field over the state an enclosing scope installed, so a
/// `.disabled(..)` inside a `.visible(..)` keeps both flags.
pub fn a11y_scoped_env_for_state_signal(
    env: &Environment,
    value: &AccessibilityStateSignal,
) -> Environment {
    let mut scoped = env.clone();
    AccessibilityStateSignal::install(&mut scoped, value.state().clone());
    scoped
}

/// The `Env` scoping for an [`AccessibilityIdentifier`]: the value wraps in a
/// [`ScopedAccessibilityIdentifier`] so the node that registers under it can
/// claim that identity rather than any other scoped identifier above it.
/// Without an accessibility tree there is nothing to name, so the value drops
/// and the environment passes through.
#[cfg(feature = "accessibility")]
pub fn a11y_scoped_identifier_env(
    env: &Environment,
    value: &AccessibilityIdentifier,
) -> Environment {
    a11y_scoped_env(env, &ScopedAccessibilityIdentifier::new(value.clone()))
}

#[cfg(not(feature = "accessibility"))]
pub fn a11y_scoped_identifier_env(
    env: &Environment,
    _value: &AccessibilityIdentifier,
) -> Environment {
    env.clone()
}

/// The `Env` scoping for [`AccessibilityChildren`]: `Combine` opens a naming
/// scope so the container (or gesture host) it decorates claims its own
/// combined element even without an explicit role or label, whereas
/// `Automatic` and `ExcludeDescendants` ride an existing scope.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "matches the uniform &T signature of accessibility_scope_table!"
)]
pub fn a11y_scoped_env_for_children<E: crate::text::TextEngine>(
    env: &Environment,
    value: &AccessibilityChildren,
) -> Environment {
    if value.combines_descendants() {
        a11y_naming_scoped_env::<AccessibilityChildren, E>(env, value)
    } else {
        a11y_scoped_env(env, value)
    }
}

/// Applies `$callback` to the accessibility metadata scopes: each key's
/// `NativeBackend` method, its type, and the function extending an
/// environment for it — the single definition of the metadata-type →
/// scoping table, so registering a new accessibility scope touches one
/// place.
/// `E` names the text engine at the expansion site.
macro_rules! accessibility_scope_table {
    ($callback:ident) => {
        $callback! {
            accessibility_label: AccessibilityLabel => a11y_naming_scoped_env::<AccessibilityLabel, E>;
            // `.a11y_value(..)` is naming metadata like the label: the node
            // that represents the wrapped view owns the semantic payload —
            // once — and a container that no control spoke for synthesizes
            // the node carrying it.
            accessibility_value: AccessibilityValue => a11y_naming_scoped_env::<AccessibilityValue, E>;
            accessibility_identifier: AccessibilityIdentifier => a11y_scoped_identifier_env;
            accessibility_role: AccessibilityRole => a11y_naming_scoped_env::<AccessibilityRole, E>;
            accessibility_hidden: AccessibilityHidden => a11y_scoped_env::<AccessibilityHidden>;
            accessibility_children: AccessibilityChildren => a11y_scoped_env_for_children::<E>;
            // The geometry source names the element the way a label does: the
            // node that represents the wrapped view reads it from the scope.
            accessibility_geometry: AccessibilityGeometry => a11y_scoped_env::<AccessibilityGeometry>;
            accessibility_state: AccessibilityState => a11y_scoped_env_for_state;
            accessibility_state_signal: AccessibilityStateSignal => a11y_scoped_env_for_state_signal;
        }
    };
}
pub(crate) use accessibility_scope_table;

/// Resolves a shape into a concrete path for `bounds`.
///
/// Structured shape kinds resolve their normalized corner radii against the
/// shorter side, producing circular corners — the same interpretation the
/// Apple backend uses. Scaling the unit-space path
/// non-uniformly instead (the `CustomPath` fallback) stretches corner arcs
/// into ellipse segments on wide containers, which violates the Material
/// corner shape (e.g. a 4dp snackbar radius smeared across a 1500px bar).
pub fn resolved_shape_to_path(shape: &ResolvedShape, bounds: kurbo::Rect) -> kurbo::BezPath {
    shape_kind_path(shape.kind(), bounds)
        .unwrap_or_else(|| path_commands_to_path(shape.commands(), bounds))
}

/// Bounds-aware path for the structured shape kinds; `None` for custom paths,
/// which only exist as unit-space commands.
pub fn shape_kind_path(kind: ShapeKind, bounds: kurbo::Rect) -> Option<kurbo::BezPath> {
    use kurbo::Shape as _;
    const PATH_TOLERANCE: f64 = 0.05;
    let min_side = crate::num_cast::f64_as_f32(bounds.width().min(bounds.height()).max(0.0));
    match kind {
        ShapeKind::Rect | ShapeKind::RoundedRect { .. } | ShapeKind::Capsule => {
            Some(rounded_rect_path(bounds, shape_kind_radii(kind, min_side)))
        }
        ShapeKind::Circle => {
            let radius = bounds.width().min(bounds.height()).max(0.0) / 2.0;
            Some(kurbo::Circle::new(bounds.center(), radius).into_path(PATH_TOLERANCE))
        }
        ShapeKind::Ellipse => Some(kurbo::Ellipse::from_rect(bounds).into_path(PATH_TOLERANCE)),
        ShapeKind::CustomPath => None,
    }
}

/// Per-corner radii in points, in top-left/top-right/bottom-right/bottom-left
/// order. Each corner resolves against `min_side` and clamps to half the
/// shorter side so radii stay absolute under `lerp`.
pub fn shape_kind_radii(kind: ShapeKind, min_side: f32) -> [f32; 4] {
    let limit = min_side / 2.0;
    match kind {
        ShapeKind::Rect => [0.0; 4],
        ShapeKind::Circle | ShapeKind::Ellipse | ShapeKind::Capsule => [limit; 4],
        ShapeKind::RoundedRect { radii } => radii.resolve_clockwise(min_side),
        ShapeKind::CustomPath => {
            unreachable!("`shape_kind_path` resolves radii only for structured kinds")
        }
    }
}

fn rounded_rect_path(bounds: kurbo::Rect, radii: [f32; 4]) -> kurbo::BezPath {
    const KAPPA: f64 = 0.552_284_749_830_793_6;
    let limit = bounds.width().min(bounds.height()).max(0.0) / 2.0;
    let [tl, tr, br, bl] = radii.map(|radius| f64::from(radius.max(0.0)).min(limit));
    let mut path = kurbo::BezPath::new();

    path.move_to((bounds.x0 + tl, bounds.y0));
    path.line_to((bounds.x1 - tr, bounds.y0));
    append_corner(
        &mut path,
        kurbo::Point::new(bounds.x1 - tr, bounds.y0 + tr),
        tr,
        -core::f64::consts::FRAC_PI_2,
        0.0,
        KAPPA,
    );
    path.line_to((bounds.x1, bounds.y1 - br));
    append_corner(
        &mut path,
        kurbo::Point::new(bounds.x1 - br, bounds.y1 - br),
        br,
        0.0,
        core::f64::consts::FRAC_PI_2,
        KAPPA,
    );
    path.line_to((bounds.x0 + bl, bounds.y1));
    append_corner(
        &mut path,
        kurbo::Point::new(bounds.x0 + bl, bounds.y1 - bl),
        bl,
        core::f64::consts::FRAC_PI_2,
        core::f64::consts::PI,
        KAPPA,
    );
    path.line_to((bounds.x0, bounds.y0 + tl));
    append_corner(
        &mut path,
        kurbo::Point::new(bounds.x0 + tl, bounds.y0 + tl),
        tl,
        core::f64::consts::PI,
        core::f64::consts::PI + core::f64::consts::FRAC_PI_2,
        KAPPA,
    );
    path.close_path();
    path
}

fn append_corner(
    path: &mut kurbo::BezPath,
    center: kurbo::Point,
    radius: f64,
    start: f64,
    end: f64,
    kappa: f64,
) {
    if radius <= 0.0 {
        return;
    }
    let start_point = kurbo::Point::new(
        radius.mul_add(start.cos(), center.x),
        radius.mul_add(start.sin(), center.y),
    );
    let end_point = kurbo::Point::new(
        radius.mul_add(end.cos(), center.x),
        radius.mul_add(end.sin(), center.y),
    );
    let c1 = kurbo::Point::new(
        (radius * kappa).mul_add(-start.sin(), start_point.x),
        (radius * kappa).mul_add(start.cos(), start_point.y),
    );
    let c2 = kurbo::Point::new(
        (radius * kappa).mul_add(end.sin(), end_point.x),
        (radius * kappa).mul_add(-end.cos(), end_point.y),
    );
    path.curve_to(c1, c2, end_point);
}

pub use crate::theme::path_commands_to_path;

pub fn anchor_point(bounds: kurbo::Rect, anchor: waterui_shape::style::Anchor) -> kurbo::Point {
    kurbo::Point::new(
        bounds.width().mul_add(f64::from(anchor.x), bounds.x0),
        bounds.height().mul_add(f64::from(anchor.y), bounds.y0),
    )
}

/// The sRGB8 encoding of a resolved working colour — the brush form text
/// shaping takes.
pub fn working_color_to_rgba8(color: cherenkov_record::WorkingColor) -> [u8; 4] {
    let srgb = waterui_graphics::color::working::to_srgb(color);
    [
        crate::num_cast::f32_as_u8((srgb.red.clamp(0.0, 1.0) * 255.0).round()),
        crate::num_cast::f32_as_u8((srgb.green.clamp(0.0, 1.0) * 255.0).round()),
        crate::num_cast::f32_as_u8((srgb.blue.clamp(0.0, 1.0) * 255.0).round()),
        crate::num_cast::f32_as_u8((color.components[3].clamp(0.0, 1.0) * 255.0).round()),
    ]
}

#[cfg_attr(hydrolysis_hwui, expect(dead_code))]
pub fn rgba8_to_peniko(color: [u8; 4]) -> peniko::Color {
    peniko::Color::new([
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    ])
}

pub fn transformed_rect(transform: kurbo::Affine, rect: kurbo::Rect) -> kurbo::Rect {
    let points = [
        transform * kurbo::Point::new(rect.x0, rect.y0),
        transform * kurbo::Point::new(rect.x1, rect.y0),
        transform * kurbo::Point::new(rect.x0, rect.y1),
        transform * kurbo::Point::new(rect.x1, rect.y1),
    ];
    let min_x = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.x));
    let min_y = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.y));
    let max_x = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.x));
    let max_y = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.y));
    kurbo::Rect::new(min_x, min_y, max_x, max_y)
}
