//! The resolved JNI surface: every class, method, and field identifier the
//! backend calls, looked up once when the runtime is created and owned by it
//! for its life.
//!
//! Two objects live here. [`Bindings`] is the `findClass`/`GetMethodID`
//! resolution table — built inside the runtime's own JNI frame, where the
//! application classloader can see `dev.waterui.android.*`, then frozen.
//! [`Platform`] adds what the table needs to run: the host `Context` the
//! views are constructed against, the display density cache, the measure
//! epoch, and the proposal channel map. Neither is queried per call
//! afterwards: the `*_unchecked` call sites take the cached identifiers
//! directly, so no string crosses JNI on the draw, measure, or watcher
//! paths.
//!
//! Nothing here is process state. The `Platform` is built inside
//! [`crate::entry::mount`], published through the `Environment`, and
//! threaded by `Rc` into every leaf, context and callback that reaches JNI.
//! The single exception is the `JavaVM` — see [`with_env`].

use core::cell::Cell;
use core::marker::PhantomData;

use jni::objects::{Global, JClass, JObject, JString, JValueOwned};
use jni::objects::{JFieldID, JMethodID, JStaticFieldID, JStaticMethodID};
use jni::signature::{JavaType, Primitive, ReturnType};
use jni::strings::{JNIStr, JNIString};
use jni::sys::{jint, jlong, jvalue};
use jni::{Env, JavaVM, jni_sig, jni_str};

use crate::measure_memo::MeasureEpoch;
use crate::proposal::Proposals;

/// `getMainLooper().isCurrentThread()` — the `MainThreadMarker` proof
/// `RenderContext` carries. `!Send` and `!Sync` by construction: created
/// once per render and passed by borrow, never moved across threads.
#[derive(Debug, Clone, Copy)]
pub struct MainThread {
    _sealed: PhantomData<*const ()>,
}

impl MainThread {
    /// Answers the proof, or `None` off the main looper — the same contract
    /// `MainThreadMarker::new` states as an `Option`.
    pub fn new(platform: &Platform) -> Option<Self> {
        with_env(|env| {
            platform
                .bindings()
                .is_current_thread(env)
                .expect("Looper.isCurrentThread is always callable")
        })
        .then_some(Self {
            _sealed: PhantomData,
        })
    }
}

/// `env` for a spot JNI call on an already-attached thread. View operations
/// are main-thread work, where the attach is a TLS check, not a handshake.
///
/// The `JavaVM` behind this is the crate's only process-global state, and it
/// has no alternative: `JNI_OnLoad` hands the machine to a C entry point
/// with no context object to thread, so `jni` itself keeps it behind
/// `JavaVM::singleton`. Everything else the backend owns — the identifier
/// table, the density, the measure epoch, the proposal channels — is
/// threaded explicitly through [`Platform`].
pub fn with_env<T>(body: impl FnOnce(&mut Env) -> T) -> T {
    JavaVM::singleton()
        .expect("JavaVM singleton is initialized by the first native call")
        .attach_current_thread(|env| -> jni::errors::Result<T> { Ok(body(env)) })
        .expect("attaching the current thread failed")
}

/// The frozen identifier table.
#[derive(Debug)]
#[allow(dead_code, reason = "some class refs only matter at resolve time")]
pub struct Bindings {
    // dev/waterui/android — the host library's bridge classes.
    rust_view_group: Global<JClass<'static>>,
    rust_view_group_ctor: JMethodID,
    rust_view_group_set_handle: JMethodID,
    rust_click_listener: Global<JClass<'static>>,
    rust_click_listener_ctor: JMethodID,

    // android/view/View — the class ref itself is only needed at resolve
    // time; instance calls go through the object.
    view_measure: JMethodID,
    view_get_measured_width: JMethodID,
    view_get_measured_height: JMethodID,
    view_layout: JMethodID,
    view_request_layout: JMethodID,
    view_set_on_click_listener: JMethodID,
    view_set_content_description: JMethodID,
    view_set_text_alignment: JMethodID,
    view_set_clickable: JMethodID,
    view_set_elevation: JMethodID,
    view_set_important_for_accessibility: JMethodID,
    view_set_background_color: JMethodID,
    view_set_clip_children: JMethodID,

    // android/view/ViewGroup.
    view_group_add_view: JMethodID,
    view_group_remove_view: JMethodID,

    // android/widget/TextView.
    text_view: Global<JClass<'static>>,
    text_view_ctor: JMethodID,
    text_view_set_text: JMethodID,
    text_view_set_text_size: JMethodID,
    text_view_set_text_color: JMethodID,
    text_view_set_max_lines: JMethodID,
    text_view_set_gravity: JMethodID,
    text_view_set_typeface: JMethodID,

    // android/widget/Space — the empty leaf's platform object.
    space: Global<JClass<'static>>,
    space_ctor: JMethodID,

    // android/widget/FrameLayout — the button chrome's wrapper — and its
    // `LayoutParams`, for match-parent + margin placement without a
    // measure pass of our own.
    frame_layout: Global<JClass<'static>>,
    frame_layout_ctor: JMethodID,
    frame_layout_params: Global<JClass<'static>>,
    frame_layout_params_ctor: JMethodID,
    layout_params_set_margins: JMethodID,
    view_set_layout_params: JMethodID,

    // android/widget/Button — the chrome behind the label view.
    button: Global<JClass<'static>>,
    button_ctor: JMethodID,

    // android/graphics/Typeface.
    typeface: Global<JClass<'static>>,
    typeface_default: JStaticFieldID,
    typeface_default_bold: JStaticFieldID,
    typeface_monospace: JStaticFieldID,

    // android/content/Context.
    context_get_resources: JMethodID,
    context_get_theme: JMethodID,
    context_get_system_service: JMethodID,

    // android/view/WindowManager + android/view/Display — the refresh rate
    // the executor's frame budget is scaled by.
    window_manager_get_default_display: JMethodID,
    display_get_refresh_rate: JMethodID,

    // android/content/res/Resources.
    resources_get_display_metrics: JMethodID,
    resources_get_configuration: JMethodID,
    resources_get_color: JMethodID,

    // android/content/res/Resources$Theme.
    theme: Global<JClass<'static>>,
    theme_resolve_attribute: JMethodID,

    // android/R$attr — the class the theme-attribute ids live on. The
    // field VALUES are resource ids assigned per platform release and
    // are not stable constants, so every attribute the theme reads is
    // looked up by name at runtime.
    r_attr: Global<JClass<'static>>,

    // android/content/res/Configuration.
    configuration_ui_mode: JFieldID,

    // android/util/DisplayMetrics.
    display_metrics_density: JFieldID,

    // android/util/TypedValue — the fields a resolved attribute is read
    // through, plus its public `TYPE_*` codes resolved by name.
    typed_value: Global<JClass<'static>>,
    typed_value_ctor: JMethodID,
    typed_value_type: JFieldID,
    typed_value_data: JFieldID,
    typed_value_resource_id: JFieldID,
    typed_value_string: JFieldID,
    typed_value_type_reference: jint,
    typed_value_type_attribute: jint,
    typed_value_type_string: jint,
    typed_value_type_first_int: jint,
    typed_value_type_last_color_int: jint,

    // android/graphics/Color — parses a literal hex color string a theme
    // may hand back for a color attribute (`TYPE_STRING` values).
    color: Global<JClass<'static>>,
    color_parse_color: JStaticMethodID,

    // java/lang/Object — `toString`, the generic read on `CharSequence`.
    object_to_string: JMethodID,

    // android/os/Looper.
    looper: Global<JClass<'static>>,
    looper_get_main_looper: JStaticMethodID,
    looper_is_current_thread: JMethodID,

    // java/util/Locale — the system locale tag.
    locale: Global<JClass<'static>>,
    locale_get_default: JStaticMethodID,
    locale_to_language_tag: JMethodID,

    // android/view/View$MeasureSpec — the packed-spec methods and the
    // public mode constants (`UNSPECIFIED`, `AT_MOST`, `EXACTLY`),
    // resolved by name like every other framework value.
    measure_spec: Global<JClass<'static>>,
    measure_spec_make: JStaticMethodID,
    measure_spec_get_mode: JStaticMethodID,
    measure_spec_get_size: JStaticMethodID,
    measure_spec_mode_unspecified: jint,
    measure_spec_mode_at_most: jint,
    measure_spec_mode_exactly: jint,

    // Framework constants read through JNI at resolve time — public
    // `static final int` values looked up by name instead of baked in.
    // android/view/Gravity.
    gravity: Global<JClass<'static>>,
    gravity_start: jint,
    gravity_center_horizontal: jint,
    gravity_end: jint,
    gravity_center_vertical: jint,
    // android/view/View.
    view_text_alignment_text_start: jint,
    view_text_alignment_center: jint,
    view_text_alignment_text_end: jint,
    view_important_for_accessibility_yes: jint,
    view_important_for_accessibility_no_hide_descendants: jint,
    // android/content/res/Configuration.
    configuration_ui_mode_night_mask: jint,
    configuration_ui_mode_night_yes: jint,
}

/// Throws `android.content.res.Resources.NotFoundException` naming the
/// `android.R.attr` field behind a theme-resolution failure — the error
/// contract theme resolution reports through. A pending Java exception
/// the failed lookup left behind is cleared first so this one lands.
fn attr_not_found(env: &mut Env, attr: &JNIStr, detail: &str) -> jni::errors::Error {
    if env.exception_check() {
        env.exception_clear();
    }
    let _ = env.throw_new(
        jni_str!("android/content/res/Resources$NotFoundException"),
        JNIString::new(alloc::format!("android.R.attr.{attr}: {detail}")),
    );
    jni::errors::Error::JavaException
}

/// Another global reference to `view` — the way a leaf shares its platform
/// object with the watcher closures and layout face that outlive it.
pub fn retain(view: &Global<JObject<'static>>) -> Global<JObject<'static>> {
    with_env(|env| {
        env.new_global_ref(view.as_ref())
            .expect("a global reference to a live object always allocates")
    })
}

/// The backend's platform-facing state, owned by the runtime and threaded
/// through every call that reaches JNI: the frozen [`Bindings`], the host
/// `Context` views are constructed against, the display density the unit
/// conversion reads, the measure epoch the memos stamp against, and the
/// proposal channel map a parent delivers selected proposals through.
///
/// The `Environment` carries one `Rc<Platform>` (installed beside the
/// dispatcher at mount) — the same channel the Apple backend uses for the
/// objects its render context and leaves share.
pub struct Platform {
    bindings: Bindings,
    /// The host activity the backend constructs views against.
    context: Global<JObject<'static>>,
    /// `DisplayMetrics.density` — px per dp; refreshed at mount and on every
    /// configuration change, so the measure path never crosses JNI to
    /// convert units. Main-thread state, so a `Cell` suffices.
    density: Cell<f32>,
    /// The measure epoch every leaf's memo stamps against — a shared cell,
    /// so `MemoizingSubView` and the invalidation callers read the same
    /// counter without a static.
    measure_epoch: MeasureEpoch,
    /// view key → the leaf's selected-proposal sink — the L-2 channel.
    /// Registration, delivery and teardown all run on the main looper.
    proposals: Proposals,
}

impl core::fmt::Debug for Platform {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Platform")
            .field("density", &self.density.get())
            .finish_non_exhaustive()
    }
}

impl Platform {
    /// Resolves every identifier once and builds the [`Platform`]. Called
    /// from `nativeCreate`'s frame — the only place the app classloader is
    /// guaranteed to resolve `dev.waterui.android.*`.
    pub fn new(env: &mut Env, activity: &JObject) -> jni::errors::Result<Self> {
        Ok(Self {
            bindings: Bindings::resolve(env)?,
            context: env.new_global_ref(activity)?,
            density: Cell::new(1.0),
            measure_epoch: MeasureEpoch::new(),
            proposals: Proposals::new(),
        })
    }

    /// The identifier table.
    pub const fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    /// The host activity — the `Context` every view constructor takes.
    pub const fn context(&self) -> &Global<JObject<'static>> {
        &self.context
    }

    /// The measure epoch the leaf memos share.
    pub fn measure_epoch(&self) -> MeasureEpoch {
        self.measure_epoch.clone()
    }

    /// Marks every memoized measure stale tree-wide — the counterpart of
    /// Apple's `MeasureMemo.invalidate`, sent through the runtime's own
    /// epoch instead of a static.
    pub fn invalidate_measures(&self) {
        self.measure_epoch.bump();
    }

    /// The proposal channel map — registers leaf sinks and delivers the
    /// proposals a parent selected, per `docs/layout-spec.md` rule L-2.
    pub const fn proposals(&self) -> &Proposals {
        &self.proposals
    }

    /// `context.getResources()`.
    fn resources(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        // SAFETY: resolved id; `context` is an Activity (a Context).
        let resources = unsafe {
            env.call_method_unchecked(
                self.context(),
                self.bindings.context_get_resources,
                ReturnType::Object,
                &[],
            )?
        };
        env.new_global_ref(resources.l()?)
    }

    /// `context.getTheme()`.
    fn theme(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        // SAFETY: resolved id; `context` is an Activity (a Context).
        let theme = unsafe {
            env.call_method_unchecked(
                self.context(),
                self.bindings.context_get_theme,
                ReturnType::Object,
                &[],
            )?
        };
        env.new_global_ref(theme.l()?)
    }

    /// Refreshes the unit cache from `DisplayMetrics` — at mount and on
    /// every `onConfigurationChanged`.
    pub fn refresh_metrics(&self, env: &mut Env) -> jni::errors::Result<()> {
        self.density.set(self.display_metrics(env)?);
        Ok(())
    }

    /// `DisplayMetrics.density` — px per dp.
    pub const fn density(&self) -> f32 {
        self.density.get()
    }

    /// dp → px, rounding to whole pixels the way `View` frames expect.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a View frame in whole pixels always fits an i32"
    )]
    pub fn dp_to_px(&self, dp: f32) -> i32 {
        (dp * self.density()).round() as i32
    }

    /// px → dp.
    #[expect(
        clippy::cast_precision_loss,
        reason = "a measured pixel size is well inside f32's exact range"
    )]
    pub fn px_to_dp(&self, px: i32) -> f32 {
        px as f32 / self.density()
    }
}

impl Bindings {
    /// The one-shot resolution — `findClass` for every class the backend
    /// touches, `GetMethodID`/`GetFieldID` for every member it calls.
    #[allow(clippy::too_many_lines, reason = "a flat id table is meant to be long")]
    fn resolve(env: &mut Env) -> jni::errors::Result<Self> {
        let mut class = |name: &JNIStr| {
            let class = env.find_class(name)?;
            env.new_global_ref(class)
        };

        let rust_view_group = class(jni_str!("dev/waterui/android/RustViewGroup"))?;
        let rust_click_listener = class(jni_str!("dev/waterui/android/RustOnClickListener"))?;
        let view = class(jni_str!("android/view/View"))?;
        let view_group = class(jni_str!("android/view/ViewGroup"))?;
        let text_view = class(jni_str!("android/widget/TextView"))?;
        let space = class(jni_str!("android/widget/Space"))?;
        let frame_layout = class(jni_str!("android/widget/FrameLayout"))?;
        let frame_layout_params = class(jni_str!("android/widget/FrameLayout$LayoutParams"))?;
        let button = class(jni_str!("android/widget/Button"))?;
        let typeface = class(jni_str!("android/graphics/Typeface"))?;
        let context = class(jni_str!("android/content/Context"))?;
        let resources = class(jni_str!("android/content/res/Resources"))?;
        let theme = class(jni_str!("android/content/res/Resources$Theme"))?;
        let configuration = class(jni_str!("android/content/res/Configuration"))?;
        let display_metrics = class(jni_str!("android/util/DisplayMetrics"))?;
        let typed_value = class(jni_str!("android/util/TypedValue"))?;
        let r_attr = class(jni_str!("android/R$attr"))?;
        let looper = class(jni_str!("android/os/Looper"))?;
        let locale = class(jni_str!("java/util/Locale"))?;
        let window_manager = class(jni_str!("android/view/WindowManager"))?;
        let display = class(jni_str!("android/view/Display"))?;
        let measure_spec = class(jni_str!("android/view/View$MeasureSpec"))?;
        let gravity = class(jni_str!("android/view/Gravity"))?;
        let color = class(jni_str!("android/graphics/Color"))?;
        let object = class(jni_str!("java/lang/Object"))?;

        // A public `static final int` on a framework class, read by name:
        // the crate never bakes a framework constant in — a wrong literal
        // is a wrong answer the compiler cannot see, and a missing field
        // fails at resolve time naming it.
        let const_int = |env: &mut Env,
                         class: &Global<JClass>,
                         name: &'static JNIStr|
         -> jni::errors::Result<jint> {
            let field = env.get_static_field_id(class, name, jni_sig!("I"))?;
            // SAFETY: resolved static int field on a framework class.
            unsafe {
                env.get_static_field_unchecked(class, field, JavaType::Primitive(Primitive::Int))?
                    .i()
            }
        };

        Ok(Self {
            rust_view_group_ctor: env.get_method_id(
                &rust_view_group,
                jni_str!("<init>"),
                jni_sig!("(Landroid/content/Context;)V"),
            )?,
            rust_view_group_set_handle: env.get_method_id(
                &rust_view_group,
                jni_str!("setHandle"),
                jni_sig!("(J)V"),
            )?,
            rust_view_group,
            rust_click_listener_ctor: env.get_method_id(
                &rust_click_listener,
                jni_str!("<init>"),
                jni_sig!("(J)V"),
            )?,
            rust_click_listener,

            view_measure: env.get_method_id(&view, jni_str!("measure"), jni_sig!("(II)V"))?,
            view_get_measured_width: env.get_method_id(
                &view,
                jni_str!("getMeasuredWidth"),
                jni_sig!("()I"),
            )?,
            view_get_measured_height: env.get_method_id(
                &view,
                jni_str!("getMeasuredHeight"),
                jni_sig!("()I"),
            )?,
            view_layout: env.get_method_id(&view, jni_str!("layout"), jni_sig!("(IIII)V"))?,
            view_request_layout: env.get_method_id(
                &view,
                jni_str!("requestLayout"),
                jni_sig!("()V"),
            )?,
            view_set_on_click_listener: env.get_method_id(
                &view,
                jni_str!("setOnClickListener"),
                jni_sig!("(Landroid/view/View$OnClickListener;)V"),
            )?,
            view_set_content_description: env.get_method_id(
                &view,
                jni_str!("setContentDescription"),
                jni_sig!("(Ljava/lang/CharSequence;)V"),
            )?,
            view_set_text_alignment: env.get_method_id(
                &view,
                jni_str!("setTextAlignment"),
                jni_sig!("(I)V"),
            )?,
            view_set_clickable: env.get_method_id(
                &view,
                jni_str!("setClickable"),
                jni_sig!("(Z)V"),
            )?,
            view_set_elevation: env.get_method_id(
                &view,
                jni_str!("setElevation"),
                jni_sig!("(F)V"),
            )?,
            view_set_important_for_accessibility: env.get_method_id(
                &view,
                jni_str!("setImportantForAccessibility"),
                jni_sig!("(I)V"),
            )?,
            view_set_background_color: env.get_method_id(
                &view,
                jni_str!("setBackgroundColor"),
                jni_sig!("(I)V"),
            )?,
            view_set_clip_children: env.get_method_id(
                &view_group,
                jni_str!("setClipChildren"),
                jni_sig!("(Z)V"),
            )?,
            view_set_layout_params: env.get_method_id(
                &view,
                jni_str!("setLayoutParams"),
                jni_sig!("(Landroid/view/ViewGroup$LayoutParams;)V"),
            )?,

            view_group_add_view: env.get_method_id(
                &view_group,
                jni_str!("addView"),
                jni_sig!("(Landroid/view/View;)V"),
            )?,
            view_group_remove_view: env.get_method_id(
                &view_group,
                jni_str!("removeView"),
                jni_sig!("(Landroid/view/View;)V"),
            )?,

            text_view_ctor: env.get_method_id(
                &text_view,
                jni_str!("<init>"),
                jni_sig!("(Landroid/content/Context;)V"),
            )?,
            text_view_set_text: env.get_method_id(
                &text_view,
                jni_str!("setText"),
                jni_sig!("(Ljava/lang/CharSequence;)V"),
            )?,
            text_view_set_text_size: env.get_method_id(
                &text_view,
                jni_str!("setTextSize"),
                jni_sig!("(F)V"),
            )?,
            text_view_set_text_color: env.get_method_id(
                &text_view,
                jni_str!("setTextColor"),
                jni_sig!("(I)V"),
            )?,
            text_view_set_max_lines: env.get_method_id(
                &text_view,
                jni_str!("setMaxLines"),
                jni_sig!("(I)V"),
            )?,
            text_view_set_gravity: env.get_method_id(
                &text_view,
                jni_str!("setGravity"),
                jni_sig!("(I)V"),
            )?,
            text_view_set_typeface: env.get_method_id(
                &text_view,
                jni_str!("setTypeface"),
                jni_sig!("(Landroid/graphics/Typeface;)V"),
            )?,
            text_view,

            space_ctor: env.get_method_id(
                &space,
                jni_str!("<init>"),
                jni_sig!("(Landroid/content/Context;)V"),
            )?,
            space,

            frame_layout_ctor: env.get_method_id(
                &frame_layout,
                jni_str!("<init>"),
                jni_sig!("(Landroid/content/Context;)V"),
            )?,
            frame_layout,

            frame_layout_params_ctor: env.get_method_id(
                &frame_layout_params,
                jni_str!("<init>"),
                jni_sig!("(II)V"),
            )?,
            layout_params_set_margins: env.get_method_id(
                &frame_layout_params,
                jni_str!("setMargins"),
                jni_sig!("(IIII)V"),
            )?,
            frame_layout_params,

            button_ctor: env.get_method_id(
                &button,
                jni_str!("<init>"),
                jni_sig!("(Landroid/content/Context;)V"),
            )?,
            button,

            typeface_default: env.get_static_field_id(
                &typeface,
                jni_str!("DEFAULT"),
                jni_sig!("Landroid/graphics/Typeface;"),
            )?,
            typeface_default_bold: env.get_static_field_id(
                &typeface,
                jni_str!("DEFAULT_BOLD"),
                jni_sig!("Landroid/graphics/Typeface;"),
            )?,
            typeface_monospace: env.get_static_field_id(
                &typeface,
                jni_str!("MONOSPACE"),
                jni_sig!("Landroid/graphics/Typeface;"),
            )?,
            typeface,

            context_get_resources: env.get_method_id(
                &context,
                jni_str!("getResources"),
                jni_sig!("()Landroid/content/res/Resources;"),
            )?,
            context_get_theme: env.get_method_id(
                &context,
                jni_str!("getTheme"),
                jni_sig!("()Landroid/content/res/Resources$Theme;"),
            )?,
            context_get_system_service: env.get_method_id(
                &context,
                jni_str!("getSystemService"),
                jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            )?,
            window_manager_get_default_display: env.get_method_id(
                &window_manager,
                jni_str!("getDefaultDisplay"),
                jni_sig!("()Landroid/view/Display;"),
            )?,
            display_get_refresh_rate: env.get_method_id(
                &display,
                jni_str!("getRefreshRate"),
                jni_sig!("()F"),
            )?,

            resources_get_display_metrics: env.get_method_id(
                &resources,
                jni_str!("getDisplayMetrics"),
                jni_sig!("()Landroid/util/DisplayMetrics;"),
            )?,
            resources_get_configuration: env.get_method_id(
                &resources,
                jni_str!("getConfiguration"),
                jni_sig!("()Landroid/content/res/Configuration;"),
            )?,
            resources_get_color: env.get_method_id(
                &resources,
                jni_str!("getColor"),
                jni_sig!("(ILandroid/content/res/Resources$Theme;)I"),
            )?,

            theme_resolve_attribute: env.get_method_id(
                &theme,
                jni_str!("resolveAttribute"),
                jni_sig!("(ILandroid/util/TypedValue;Z)Z"),
            )?,
            theme,
            r_attr,

            configuration_ui_mode: env.get_field_id(
                &configuration,
                jni_str!("uiMode"),
                jni_sig!("I"),
            )?,

            display_metrics_density: env.get_field_id(
                &display_metrics,
                jni_str!("density"),
                jni_sig!("F"),
            )?,

            typed_value_ctor: env.get_method_id(
                &typed_value,
                jni_str!("<init>"),
                jni_sig!("()V"),
            )?,
            typed_value_type: env.get_field_id(&typed_value, jni_str!("type"), jni_sig!("I"))?,
            typed_value_data: env.get_field_id(&typed_value, jni_str!("data"), jni_sig!("I"))?,
            typed_value_resource_id: env.get_field_id(
                &typed_value,
                jni_str!("resourceId"),
                jni_sig!("I"),
            )?,
            typed_value_string: env.get_field_id(
                &typed_value,
                jni_str!("string"),
                jni_sig!("Ljava/lang/CharSequence;"),
            )?,
            typed_value_type_reference: const_int(env, &typed_value, jni_str!("TYPE_REFERENCE"))?,
            typed_value_type_attribute: const_int(env, &typed_value, jni_str!("TYPE_ATTRIBUTE"))?,
            typed_value_type_string: const_int(env, &typed_value, jni_str!("TYPE_STRING"))?,
            typed_value_type_first_int: const_int(env, &typed_value, jni_str!("TYPE_FIRST_INT"))?,
            typed_value_type_last_color_int: const_int(
                env,
                &typed_value,
                jni_str!("TYPE_LAST_COLOR_INT"),
            )?,
            typed_value,

            color_parse_color: env.get_static_method_id(
                &color,
                jni_str!("parseColor"),
                jni_sig!("(Ljava/lang/String;)I"),
            )?,
            color,

            object_to_string: env.get_method_id(
                &object,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
            )?,

            looper_get_main_looper: env.get_static_method_id(
                &looper,
                jni_str!("getMainLooper"),
                jni_sig!("()Landroid/os/Looper;"),
            )?,
            looper_is_current_thread: env.get_method_id(
                &looper,
                jni_str!("isCurrentThread"),
                jni_sig!("()Z"),
            )?,
            looper,

            locale_get_default: env.get_static_method_id(
                &locale,
                jni_str!("getDefault"),
                jni_sig!("()Ljava/util/Locale;"),
            )?,
            locale_to_language_tag: env.get_method_id(
                &locale,
                jni_str!("toLanguageTag"),
                jni_sig!("()Ljava/lang/String;"),
            )?,
            locale,

            measure_spec_make: env.get_static_method_id(
                &measure_spec,
                jni_str!("makeMeasureSpec"),
                jni_sig!("(II)I"),
            )?,
            measure_spec_get_mode: env.get_static_method_id(
                &measure_spec,
                jni_str!("getMode"),
                jni_sig!("(I)I"),
            )?,
            measure_spec_get_size: env.get_static_method_id(
                &measure_spec,
                jni_str!("getSize"),
                jni_sig!("(I)I"),
            )?,
            measure_spec_mode_unspecified: const_int(env, &measure_spec, jni_str!("UNSPECIFIED"))?,
            measure_spec_mode_at_most: const_int(env, &measure_spec, jni_str!("AT_MOST"))?,
            measure_spec_mode_exactly: const_int(env, &measure_spec, jni_str!("EXACTLY"))?,
            measure_spec,

            gravity_start: const_int(env, &gravity, jni_str!("START"))?,
            gravity_center_horizontal: const_int(env, &gravity, jni_str!("CENTER_HORIZONTAL"))?,
            gravity_end: const_int(env, &gravity, jni_str!("END"))?,
            gravity_center_vertical: const_int(env, &gravity, jni_str!("CENTER_VERTICAL"))?,
            gravity,
            view_text_alignment_text_start: const_int(
                env,
                &view,
                jni_str!("TEXT_ALIGNMENT_TEXT_START"),
            )?,
            view_text_alignment_center: const_int(env, &view, jni_str!("TEXT_ALIGNMENT_CENTER"))?,
            view_text_alignment_text_end: const_int(
                env,
                &view,
                jni_str!("TEXT_ALIGNMENT_TEXT_END"),
            )?,
            view_important_for_accessibility_yes: const_int(
                env,
                &view,
                jni_str!("IMPORTANT_FOR_ACCESSIBILITY_YES"),
            )?,
            view_important_for_accessibility_no_hide_descendants: const_int(
                env,
                &view,
                jni_str!("IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS"),
            )?,
            configuration_ui_mode_night_mask: const_int(
                env,
                &configuration,
                jni_str!("UI_MODE_NIGHT_MASK"),
            )?,
            configuration_ui_mode_night_yes: const_int(
                env,
                &configuration,
                jni_str!("UI_MODE_NIGHT_YES"),
            )?,
        })
    }
}

impl Platform {
    /// A `View` subclass' platform object, constructed against the host
    /// context and returned as a [`Global`]: the leaf owns it, exactly like
    /// the retained object an Apple leaf holds.
    fn construct(
        &self,
        env: &mut Env,
        class: &Global<JClass<'static>>,
        ctor: JMethodID,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        // SAFETY: `class`/`ctor` come from the resolved table, and the
        // constructor signature is the shared `(Context)` shape every `View`
        // subclass declares — the only argument is the host context.
        let object = unsafe {
            env.new_object_unchecked(
                class,
                ctor,
                &[jvalue {
                    l: self.context().as_raw(),
                }],
            )?
        };
        env.new_global_ref(&object)
    }

    /// `new RustViewGroup(context)`.
    pub fn new_rust_view_group(
        &self,
        env: &mut Env,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        self.construct(
            env,
            &self.bindings.rust_view_group,
            self.bindings.rust_view_group_ctor,
        )
    }

    /// `new Space(context)` — the empty leaf.
    pub fn new_space(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        self.construct(env, &self.bindings.space, self.bindings.space_ctor)
    }

    /// `new TextView(context)`.
    pub fn new_text_view(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        self.construct(env, &self.bindings.text_view, self.bindings.text_view_ctor)
    }

    /// `new FrameLayout(context)`.
    pub fn new_frame_layout(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        self.construct(
            env,
            &self.bindings.frame_layout,
            self.bindings.frame_layout_ctor,
        )
    }

    /// `new Button(context)` — the chrome a button leaf fills.
    pub fn new_button(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        self.construct(env, &self.bindings.button, self.bindings.button_ctor)
    }
}

impl Bindings {
    /// `new RustOnClickListener(handle)`.
    pub fn new_click_listener(
        &self,
        env: &mut Env,
        handle: jlong,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        // SAFETY: resolved class and constructor `(J)V`.
        let object = unsafe {
            env.new_object_unchecked(
                &self.rust_click_listener,
                self.rust_click_listener_ctor,
                &[jvalue { j: handle }],
            )?
        };
        env.new_global_ref(&object)
    }

    /// `group.setHandle(handle)` — the `ContainerState` the Kotlin bridge
    /// forwards `onMeasure`/`onLayout` to.
    pub fn set_handle(
        &self,
        env: &mut Env,
        group: &JObject,
        handle: jlong,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id on the resolved class; `group` is a
        // RustViewGroup instance.
        unsafe {
            env.call_method_unchecked(
                group,
                self.rust_view_group_set_handle,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { j: handle }],
            )?;
        }
        Ok(())
    }

    /// `view.measure(widthSpec, heightSpec)` — a platform measure against
    /// explicit specs, the probe every `ViewSubView` runs.
    pub fn measure(
        &self,
        env: &mut Env,
        view: &JObject,
        width_spec: jint,
        height_spec: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_measure,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: width_spec }, jvalue { i: height_spec }],
            )?;
        }
        Ok(())
    }

    /// `(view.getMeasuredWidth(), view.getMeasuredHeight())` in pixels.
    pub fn measured_size(
        &self,
        env: &mut Env,
        view: &JObject,
    ) -> jni::errors::Result<(jint, jint)> {
        // SAFETY: resolved ids; `view` is a measured View.
        let (width, height) = unsafe {
            (
                env.call_method_unchecked(
                    view,
                    self.view_get_measured_width,
                    ReturnType::Primitive(Primitive::Int),
                    &[],
                )?,
                env.call_method_unchecked(
                    view,
                    self.view_get_measured_height,
                    ReturnType::Primitive(Primitive::Int),
                    &[],
                )?,
            )
        };
        Ok((width.i()?, height.i()?))
    }

    /// `view.layout(l, t, r, b)` — the placement a container writes.
    ///
    /// The write is the leaf's whole layout pass: `measure` runs first with
    /// `EXACTLY` specs matching the frame. A `ViewGroup` leaf (the button
    /// shell, a `Dynamic` host) places children in `onLayout` from
    /// `getMeasuredWidth`/`getMeasuredHeight`, which stay zero on a view
    /// that only ever saw `layout()`; a plain leaf measures to the same
    /// frame Rust is about to set, so the extra call is a no-op.
    pub fn layout(
        &self,
        env: &mut Env,
        view: &JObject,
        left: jint,
        top: jint,
        right: jint,
        bottom: jint,
    ) -> jni::errors::Result<()> {
        let exactly = self.measure_spec_exactly();
        let width_spec = self.make_measure_spec(env, right - left, exactly)?;
        let height_spec = self.make_measure_spec(env, bottom - top, exactly)?;
        self.measure(env, view, width_spec, height_spec)?;
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_layout,
                ReturnType::Primitive(Primitive::Void),
                &[
                    jvalue { i: left },
                    jvalue { i: top },
                    jvalue { i: right },
                    jvalue { i: bottom },
                ],
            )?;
        }
        Ok(())
    }

    /// `view.requestLayout()` — the dirty mark reactive changes set.
    pub fn request_layout(&self, env: &mut Env, view: &JObject) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_request_layout,
                ReturnType::Primitive(Primitive::Void),
                &[],
            )?;
        }
        Ok(())
    }

    /// `parent.addView(child)` — `NativeLeaf::mount`.
    pub fn add_view(
        &self,
        env: &mut Env,
        parent: &JObject,
        child: &JObject,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `parent` is a ViewGroup, `child` a View.
        unsafe {
            env.call_method_unchecked(
                parent,
                self.view_group_add_view,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { l: child.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `parent.removeView(child)` — `Mounted`'s detach.
    pub fn remove_view(
        &self,
        env: &mut Env,
        parent: &JObject,
        child: &JObject,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `parent` is a ViewGroup, `child` a View.
        unsafe {
            env.call_method_unchecked(
                parent,
                self.view_group_remove_view,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { l: child.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `view.setOnClickListener(listener)` — `listener` may be null to clear.
    pub fn set_on_click_listener(
        &self,
        env: &mut Env,
        view: &JObject,
        listener: Option<&JObject>,
    ) -> jni::errors::Result<()> {
        let raw = listener.map_or_else(core::ptr::null_mut, JObject::as_raw);
        // SAFETY: resolved id; `view` is a View; null clears the listener.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_on_click_listener,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { l: raw }],
            )?;
        }
        Ok(())
    }

    /// `view.setContentDescription(text)` — the spoken label.
    pub fn set_content_description(
        &self,
        env: &mut Env,
        view: &JObject,
        text: &JString,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View; a JString is a CharSequence.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_content_description,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { l: text.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `view.setBackgroundColor(argb)` — packed ARGB, for the window's
    /// resolved background behind the mounted content.
    pub fn set_background_color(
        &self,
        env: &mut Env,
        view: &JObject,
        argb: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_background_color,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: argb }],
            )?;
        }
        Ok(())
    }

    /// `viewGroup.setClipChildren(clip)` — off by default so a child may
    /// draw past its bounds the way Flutter lets it.
    pub fn set_clip_children(
        &self,
        env: &mut Env,
        group: &JObject,
        clip: bool,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `group` is a ViewGroup.
        unsafe {
            env.call_method_unchecked(
                group,
                self.view_set_clip_children,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { z: clip }],
            )?;
        }
        Ok(())
    }

    /// `new FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT)` — a
    /// `FrameLayout` child that fills its parent.
    pub fn new_match_parent_params(
        &self,
        env: &mut Env,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        // SAFETY: resolved `(II)V` ctor on FrameLayout.LayoutParams.
        let params = unsafe {
            env.new_object_unchecked(
                &self.frame_layout_params,
                self.frame_layout_params_ctor,
                &[jvalue { i: -1 }, jvalue { i: -1 }],
            )?
        };
        env.new_global_ref(params)
    }

    /// `params.setMargins(l, t, r, b)` in px.
    pub fn set_margins(
        &self,
        env: &mut Env,
        params: &JObject,
        left: jint,
        top: jint,
        right: jint,
        bottom: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `params` is a MarginLayoutParams.
        unsafe {
            env.call_method_unchecked(
                params,
                self.layout_params_set_margins,
                ReturnType::Primitive(Primitive::Void),
                &[
                    jvalue { i: left },
                    jvalue { i: top },
                    jvalue { i: right },
                    jvalue { i: bottom },
                ],
            )?;
        }
        Ok(())
    }

    /// `view.setLayoutParams(params)`.
    pub fn set_layout_params(
        &self,
        env: &mut Env,
        view: &JObject,
        params: &JObject,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `params` is a ViewGroup.LayoutParams.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_layout_params,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { l: params.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `textView.setText(text)`.
    pub fn set_text(
        &self,
        env: &mut Env,
        text_view: &JObject,
        text: &JString,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `text_view` is a TextView.
        unsafe {
            env.call_method_unchecked(
                text_view,
                self.text_view_set_text,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { l: text.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `textView.setTextSize(size)` — the single-float overload, which
    /// already reads the argument as scale-independent pixels.
    pub fn set_text_size(
        &self,
        env: &mut Env,
        text_view: &JObject,
        size_sp: f32,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `text_view` is a TextView.
        unsafe {
            env.call_method_unchecked(
                text_view,
                self.text_view_set_text_size,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { f: size_sp }],
            )?;
        }
        Ok(())
    }

    /// `textView.setTextColor(argb)` — packed ARGB.
    pub fn set_text_color(
        &self,
        env: &mut Env,
        text_view: &JObject,
        argb: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `text_view` is a TextView.
        unsafe {
            env.call_method_unchecked(
                text_view,
                self.text_view_set_text_color,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: argb }],
            )?;
        }
        Ok(())
    }

    /// `textView.setMaxLines(lines)`.
    pub fn set_max_lines(
        &self,
        env: &mut Env,
        text_view: &JObject,
        lines: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `text_view` is a TextView.
        unsafe {
            env.call_method_unchecked(
                text_view,
                self.text_view_set_max_lines,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: lines }],
            )?;
        }
        Ok(())
    }

    /// `textView.setGravity(gravity)` — `android.view.Gravity` flags.
    pub fn set_gravity(
        &self,
        env: &mut Env,
        text_view: &JObject,
        gravity: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `text_view` is a TextView.
        unsafe {
            env.call_method_unchecked(
                text_view,
                self.text_view_set_gravity,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: gravity }],
            )?;
        }
        Ok(())
    }

    /// `textView.setTypeface(typeface)`.
    pub fn set_typeface(
        &self,
        env: &mut Env,
        text_view: &JObject,
        typeface: &JObject,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `text_view` is a TextView.
        unsafe {
            env.call_method_unchecked(
                text_view,
                self.text_view_set_typeface,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue {
                    l: typeface.as_raw(),
                }],
            )?;
        }
        Ok(())
    }

    /// `view.setTextAlignment(alignment)` — `View.TEXT_ALIGNMENT_*`.
    pub fn set_text_alignment(
        &self,
        env: &mut Env,
        view: &JObject,
        alignment: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_text_alignment,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: alignment }],
            )?;
        }
        Ok(())
    }

    /// `view.setClickable(clickable)` — a label drawn above chrome must not
    /// swallow the press it decorates.
    pub fn set_clickable(
        &self,
        env: &mut Env,
        view: &JObject,
        clickable: bool,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_clickable,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { z: clickable }],
            )?;
        }
        Ok(())
    }

    /// `view.setElevation(px)` — pixels, not dp.
    pub fn set_elevation(&self, env: &mut Env, view: &JObject, px: f32) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_elevation,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { f: px }],
            )?;
        }
        Ok(())
    }

    /// `view.setImportantForAccessibility(mode)` —
    /// `View.IMPORTANT_FOR_ACCESSIBILITY_*`.
    pub fn set_important_for_accessibility(
        &self,
        env: &mut Env,
        view: &JObject,
        mode: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_set_important_for_accessibility,
                ReturnType::Primitive(Primitive::Void),
                &[jvalue { i: mode }],
            )?;
        }
        Ok(())
    }

    /// `Typeface` static field read (`DEFAULT`, `DEFAULT_BOLD`, `MONOSPACE`).
    pub fn typeface(
        &self,
        env: &mut Env,
        face: Face,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        let field = match face {
            Face::Default => self.typeface_default,
            Face::DefaultBold => self.typeface_default_bold,
            Face::Monospace => self.typeface_monospace,
        };
        // SAFETY: resolved static field on the resolved class.
        let value =
            unsafe { env.get_static_field_unchecked(&self.typeface, field, JavaType::Object)? };
        env.new_global_ref(value.l()?)
    }

    /// `Looper.getMainLooper().isCurrentThread()` — the `MainThread` proof.
    pub fn is_current_thread(&self, env: &mut Env) -> jni::errors::Result<bool> {
        // SAFETY: resolved static method on Looper.
        let main = unsafe {
            env.call_static_method_unchecked(
                &self.looper,
                self.looper_get_main_looper,
                ReturnType::Object,
                &[],
            )?
        };
        let main = main.l()?;
        // SAFETY: resolved method; `main` is the process's main Looper.
        let current = unsafe {
            env.call_method_unchecked(
                &main,
                self.looper_is_current_thread,
                ReturnType::Primitive(Primitive::Boolean),
                &[],
            )?
        };
        current.z()
    }

    /// `Locale.getDefault().toLanguageTag()` — the platform's preferred
    /// locale as a BCP 47 tag.
    pub fn locale_tag(&self, env: &mut Env) -> jni::errors::Result<String> {
        // SAFETY: resolved static method on Locale.
        let locale = unsafe {
            env.call_static_method_unchecked(
                &self.locale,
                self.locale_get_default,
                ReturnType::Object,
                &[],
            )?
        };
        let locale = locale.l()?;
        // SAFETY: resolved method; `locale` is a Locale.
        let tag = unsafe {
            env.call_method_unchecked(
                &locale,
                self.locale_to_language_tag,
                ReturnType::Object,
                &[],
            )?
        };
        let tag: JString = env.cast_local::<JString>(tag.l()?)?;
        tag.try_to_string(env)
    }

    /// `Object.toString` — the generic read on any `CharSequence` or
    /// object a field hands back.
    pub fn to_string(&self, env: &mut Env, object: &JObject) -> jni::errors::Result<String> {
        // SAFETY: resolved method; every object answers `toString`.
        let s = unsafe {
            env.call_method_unchecked(object, self.object_to_string, ReturnType::Object, &[])?
        };
        let s: JString = env.cast_local::<JString>(s.l()?)?;
        s.try_to_string(env)
    }

    /// `TypedValue.string` as a Rust `String` — the text a `TYPE_STRING`
    /// value carries (a literal hex color or a resource path).
    pub fn typed_value_string(
        &self,
        env: &mut Env,
        value: &JObject,
    ) -> jni::errors::Result<String> {
        // SAFETY: resolved field on a TypedValue; `string` is a
        // CharSequence the field sig declares.
        let chars =
            unsafe { env.get_field_unchecked(value, self.typed_value_string, JavaType::Object)? };
        self.to_string(env, &chars.l()?)
    }

    /// `Color.parseColor(text)` — a theme-resolved color string as ARGB;
    /// `Err` surfaces the platform's `IllegalArgumentException` for a
    /// string that names no color.
    pub fn parse_color(&self, env: &mut Env, text: &str) -> jni::errors::Result<jint> {
        let text = env.new_string(text)?;
        // SAFETY: resolved static method; the arg is a String.
        unsafe {
            env.call_static_method_unchecked(
                &self.color,
                self.color_parse_color,
                ReturnType::Primitive(Primitive::Int),
                &[jvalue { l: text.as_raw() }],
            )?
            .i()
        }
    }

    /// `View.MeasureSpec.makeMeasureSpec(size, mode)` — the packed spec.
    pub fn make_measure_spec(
        &self,
        env: &mut Env,
        size: jint,
        mode: jint,
    ) -> jni::errors::Result<jint> {
        // SAFETY: resolved static method on View$MeasureSpec.
        let spec = unsafe {
            env.call_static_method_unchecked(
                &self.measure_spec,
                self.measure_spec_make,
                ReturnType::Primitive(Primitive::Int),
                &[jvalue { i: size }, jvalue { i: mode }],
            )?
        };
        spec.i()
    }

    /// `View.MeasureSpec.getMode(spec)`.
    pub fn measure_spec_mode(&self, env: &mut Env, spec: jint) -> jni::errors::Result<jint> {
        // SAFETY: resolved static method on View$MeasureSpec.
        let mode = unsafe {
            env.call_static_method_unchecked(
                &self.measure_spec,
                self.measure_spec_get_mode,
                ReturnType::Primitive(Primitive::Int),
                &[jvalue { i: spec }],
            )?
        };
        mode.i()
    }

    /// `View.MeasureSpec.getSize(spec)`.
    pub fn measure_spec_size(&self, env: &mut Env, spec: jint) -> jni::errors::Result<jint> {
        // SAFETY: resolved static method on View$MeasureSpec.
        let size = unsafe {
            env.call_static_method_unchecked(
                &self.measure_spec,
                self.measure_spec_get_size,
                ReturnType::Primitive(Primitive::Int),
                &[jvalue { i: spec }],
            )?
        };
        size.i()
    }

    /// `View.MeasureSpec.UNSPECIFIED` — resolved at startup, not baked in.
    pub const fn measure_spec_unspecified(&self) -> jint {
        self.measure_spec_mode_unspecified
    }

    /// `View.MeasureSpec.AT_MOST`.
    pub const fn measure_spec_at_most(&self) -> jint {
        self.measure_spec_mode_at_most
    }

    /// `View.MeasureSpec.EXACTLY`.
    pub const fn measure_spec_exactly(&self) -> jint {
        self.measure_spec_mode_exactly
    }

    /// `TypedValue.TYPE_REFERENCE` — resolved at startup, not baked in.
    pub const fn typed_value_type_reference(&self) -> jint {
        self.typed_value_type_reference
    }

    /// `TypedValue.TYPE_ATTRIBUTE`.
    pub const fn typed_value_type_attribute(&self) -> jint {
        self.typed_value_type_attribute
    }

    /// `TypedValue.TYPE_STRING`.
    pub const fn typed_value_type_string(&self) -> jint {
        self.typed_value_type_string
    }

    /// `TypedValue.TYPE_FIRST_INT` — the low end of the direct-int range
    /// (`data` carries the value).
    pub const fn typed_value_type_first_int(&self) -> jint {
        self.typed_value_type_first_int
    }

    /// `TypedValue.TYPE_LAST_COLOR_INT` — the high end of it.
    pub const fn typed_value_type_last_color_int(&self) -> jint {
        self.typed_value_type_last_color_int
    }

    /// `Gravity.START` — resolved at startup, not baked in.
    pub const fn gravity_start(&self) -> jint {
        self.gravity_start
    }

    /// `Gravity.CENTER_HORIZONTAL`.
    pub const fn gravity_center_horizontal(&self) -> jint {
        self.gravity_center_horizontal
    }

    /// `Gravity.END`.
    pub const fn gravity_end(&self) -> jint {
        self.gravity_end
    }

    /// `Gravity.CENTER_VERTICAL`.
    pub const fn gravity_center_vertical(&self) -> jint {
        self.gravity_center_vertical
    }

    /// `View.TEXT_ALIGNMENT_TEXT_START`.
    pub const fn text_alignment_text_start(&self) -> jint {
        self.view_text_alignment_text_start
    }

    /// `View.TEXT_ALIGNMENT_CENTER`.
    pub const fn text_alignment_center(&self) -> jint {
        self.view_text_alignment_center
    }

    /// `View.TEXT_ALIGNMENT_TEXT_END`.
    pub const fn text_alignment_text_end(&self) -> jint {
        self.view_text_alignment_text_end
    }

    /// `View.IMPORTANT_FOR_ACCESSIBILITY_YES`.
    pub const fn important_for_accessibility_yes(&self) -> jint {
        self.view_important_for_accessibility_yes
    }

    /// `View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS`.
    pub const fn important_for_accessibility_no_hide_descendants(&self) -> jint {
        self.view_important_for_accessibility_no_hide_descendants
    }

    /// `Configuration.UI_MODE_NIGHT_MASK`.
    pub const fn ui_mode_night_mask(&self) -> jint {
        self.configuration_ui_mode_night_mask
    }

    /// `Configuration.UI_MODE_NIGHT_YES`.
    pub const fn ui_mode_night_yes(&self) -> jint {
        self.configuration_ui_mode_night_yes
    }
}

impl Platform {
    /// `resources.getDisplayMetrics().density` — px per dp.
    pub fn display_metrics(&self, env: &mut Env) -> jni::errors::Result<f32> {
        let resources = self.resources(env)?;
        // SAFETY: resolved id; `resources` is a Resources.
        let metrics = unsafe {
            env.call_method_unchecked(
                &resources,
                self.bindings.resources_get_display_metrics,
                ReturnType::Object,
                &[],
            )?
        };
        let metrics = metrics.l()?;
        // SAFETY: resolved fields on DisplayMetrics.
        let density = unsafe {
            env.get_field_unchecked(
                &metrics,
                self.bindings.display_metrics_density,
                JavaType::Primitive(Primitive::Float),
            )?
        };
        density.f()
    }

    /// `resources.getConfiguration().uiMode` — raw `Configuration.uiMode`.
    pub fn ui_mode(&self, env: &mut Env) -> jni::errors::Result<jint> {
        let resources = self.resources(env)?;
        // SAFETY: resolved id; `resources` is a Resources.
        let configuration = unsafe {
            env.call_method_unchecked(
                &resources,
                self.bindings.resources_get_configuration,
                ReturnType::Object,
                &[],
            )?
        };
        let configuration = configuration.l()?;
        // SAFETY: resolved field on Configuration.
        let ui_mode = unsafe {
            env.get_field_unchecked(
                &configuration,
                self.bindings.configuration_ui_mode,
                JavaType::Primitive(Primitive::Int),
            )?
        };
        ui_mode.i()
    }

    /// `android.R.attr.<name>` — the theme attribute's framework id,
    /// looked up by reflection at runtime. `android.R.attr.*` values
    /// are resource ids assigned per platform release, never stable
    /// constants, so the crate names every attribute it reads and
    /// resolves the ids here instead of baking them in.
    ///
    /// # Errors
    ///
    /// A pending `Resources.NotFoundException` naming the attribute
    /// when the platform defines no such field.
    pub fn framework_attr(
        &self,
        env: &mut Env,
        name: &'static JNIStr,
    ) -> jni::errors::Result<jint> {
        match env.get_static_field_id(&self.bindings.r_attr, name, jni_sig!("I")) {
            // SAFETY: resolved static int field on android.R.attr.
            Ok(field) => unsafe {
                env.get_static_field_unchecked(
                    &self.bindings.r_attr,
                    field,
                    JavaType::Primitive(Primitive::Int),
                )?
                .i()
            },
            Err(_) => Err(attr_not_found(
                env,
                name,
                "the platform defines no such attribute",
            )),
        }
    }

    /// `theme.resolveAttribute(attr, typedValue, true)` +
    /// `resources.getColor(resolvedId, theme)` — a themed color as ARGB.
    ///
    /// # Errors
    ///
    /// A pending `Resources.NotFoundException` naming `android.R.attr`
    /// `<attr_name>` when the attribute does not resolve to a color.
    pub fn theme_color(
        &self,
        env: &mut Env,
        attr: jint,
        attr_name: &'static JNIStr,
    ) -> jni::errors::Result<jint> {
        let theme = self.theme(env)?;
        // SAFETY: resolved constructor; `value` is a fresh TypedValue the
        // resolve call fills by contract.
        let value = unsafe {
            env.new_object_unchecked(
                &self.bindings.typed_value,
                self.bindings.typed_value_ctor,
                &[],
            )?
        };
        // SAFETY: resolved method; `value` is a TypedValue.
        let resolved = unsafe {
            env.call_method_unchecked(
                &theme,
                self.bindings.theme_resolve_attribute,
                ReturnType::Primitive(Primitive::Boolean),
                &[
                    jvalue { i: attr },
                    jvalue { l: value.as_raw() },
                    jvalue { z: true },
                ],
            )?
        };
        if !resolved.z()? {
            return Err(attr_not_found(
                env,
                attr_name,
                "the activity theme does not resolve it",
            ));
        }
        self.theme_color_from_value(env, &value, attr_name)
    }

    /// The ARGB a resolved `TypedValue` carries for `attr_name`,
    /// interpreted the way `ResourcesImpl.getColor` reads a theme color
    /// attribute: an int-typed value (`TYPE_FIRST_INT` through
    /// `TYPE_LAST_COLOR_INT`) carries the ARGB in `data`; a
    /// `TYPE_REFERENCE` names the color resource in `data` —
    /// `resourceId` is the resource the value was *declared in*, on API
    /// 36 the framework style carrying the theme assignment, and feeding
    /// it to `getColor` throws `NotFoundException` on a style id; a
    /// `TYPE_STRING` value is a literal color string to parse; a
    /// `TYPE_ATTRIBUTE` maps the attribute onto another attribute and
    /// resolves one hop deeper. Anything else is not a color.
    fn theme_color_from_value(
        &self,
        env: &mut Env,
        value: &JObject,
        attr_name: &'static JNIStr,
    ) -> jni::errors::Result<jint> {
        let bindings = &self.bindings;
        // SAFETY: resolved fields on the TypedValue `resolveAttribute` filled.
        let (kind, data, resource_id) = unsafe {
            (
                env.get_field_unchecked(
                    value,
                    bindings.typed_value_type,
                    JavaType::Primitive(Primitive::Int),
                )?
                .i()?,
                env.get_field_unchecked(
                    value,
                    bindings.typed_value_data,
                    JavaType::Primitive(Primitive::Int),
                )?
                .i()?,
                env.get_field_unchecked(
                    value,
                    bindings.typed_value_resource_id,
                    JavaType::Primitive(Primitive::Int),
                )?
                .i()?,
            )
        };
        if (bindings.typed_value_type_first_int()..=bindings.typed_value_type_last_color_int())
            .contains(&kind)
        {
            return Ok(data);
        }
        if kind == bindings.typed_value_type_attribute() {
            return self.theme_color(env, data, attr_name);
        }
        if kind == bindings.typed_value_type_string() {
            // Two shapes land here: a literal color string (`"#33FF…"`,
            // resourceId 0) to parse, or a resource path
            // ("res/color/x.xml") whose res id `resourceId` already
            // carries — the id `getColor` wants.
            if resource_id != 0 {
                return self.resource_color(env, resource_id, attr_name, 0);
            }
            let text = bindings.typed_value_string(env, value)?;
            return bindings.parse_color(env, &text).map_err(|_| {
                attr_not_found(
                    env,
                    attr_name,
                    &alloc::format!("it resolves to the string \"{text}\", which is not a color"),
                )
            });
        }
        if kind != bindings.typed_value_type_reference() {
            return Err(attr_not_found(
                env,
                attr_name,
                &alloc::format!(
                    "the theme resolves it to a TypedValue of type {kind} \
                     (data 0x{data:08x}, resourceId 0x{resource_id:08x}), not a color",
                ),
            ));
        }
        self.resource_color(env, data, attr_name, resource_id)
    }

    /// `resources.getColor(res_id, theme)` — a themed color resource as
    /// ARGB, with `declared_in` naming the resource the value was
    /// declared in for the failure message.
    fn resource_color(
        &self,
        env: &mut Env,
        res_id: jint,
        attr_name: &'static JNIStr,
        declared_in: jint,
    ) -> jni::errors::Result<jint> {
        let theme = self.theme(env)?;
        let resources = self.resources(env)?;
        // SAFETY: resolved method; `res_id` names a color resource.
        let color = unsafe {
            env.call_method_unchecked(
                &resources,
                self.bindings.resources_get_color,
                ReturnType::Primitive(Primitive::Int),
                &[jvalue { i: res_id }, jvalue { l: theme.as_raw() }],
            )
        };
        color.map_or_else(
            |_| {
                Err(attr_not_found(
                    env,
                    attr_name,
                    &alloc::format!(
                        "it resolves to resource 0x{res_id:08x} \
                         (declared in 0x{declared_in:08x}), which is not a color",
                    ),
                ))
            },
            JValueOwned::i,
        )
    }

    /// `context.getSystemService("window").getDefaultDisplay()
    /// .getRefreshRate()` — the display's nominal refresh in Hz, for the
    /// executor's frame budget; `None` when the platform reports none.
    pub fn refresh_rate_hz(&self, env: &mut Env) -> jni::errors::Result<Option<f32>> {
        let name = env.new_string("window")?;
        // SAFETY: resolved method; `context` is an Activity.
        let service = unsafe {
            env.call_method_unchecked(
                self.context(),
                self.bindings.context_get_system_service,
                ReturnType::Object,
                &[jvalue { l: name.as_raw() }],
            )?
        };
        let service = service.l()?;
        if service.is_null() {
            return Ok(None);
        }
        // SAFETY: resolved method; `service` is the WindowManager.
        let display = unsafe {
            env.call_method_unchecked(
                &service,
                self.bindings.window_manager_get_default_display,
                ReturnType::Object,
                &[],
            )?
        };
        let display = display.l()?;
        if display.is_null() {
            return Ok(None);
        }
        // SAFETY: resolved method; `display` is a Display.
        let rate = unsafe {
            env.call_method_unchecked(
                &display,
                self.bindings.display_get_refresh_rate,
                ReturnType::Primitive(Primitive::Float),
                &[],
            )?
        };
        let hz = rate.f()?;
        // `getRefreshRate` reports 0 when no rate is known; a non-positive
        // answer is "unavailable", not a budget.
        Ok((hz > 0.0).then_some(hz))
    }
}

/// The `Typeface` faces the text port picks between — a semantic request,
/// not a family name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    /// The platform's default proportional face.
    Default,
    /// The platform's bold face for `FontWeight::SemiBold` and up.
    DefaultBold,
    /// The platform's fixed-pitch face for `FontDesign::Monospaced`.
    Monospace,
}
