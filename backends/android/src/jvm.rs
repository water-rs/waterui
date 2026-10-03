//! The resolved JNI surface: every class, method, and field identifier the
//! backend calls, looked up once when the runtime is created and held for the
//! process's life.
//!
//! Two objects live here. [`Bindings`] is the `findClass`/`GetMethodID`
//! resolution table — built inside the runtime's own JNI frame, where the
//! application classloader can see `dev.waterui.android.*`, then frozen.
//! [`Globals`] adds the host `Context` the views are constructed against.
//! Neither is queried per call afterwards: the `*_unchecked` call sites take
//! the cached identifiers directly, so no string crosses JNI on the draw,
//! measure, or watcher paths.

use core::marker::PhantomData;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use jni::objects::{Global, JClass, JObject, JString};
use jni::signature::{JavaType, Primitive, ReturnType};
use jni::sys::{jint, jlong, jvalue};
use jni::{Env, JavaVM, jni_sig, jni_str};
use jni::objects::{JFieldID, JMethodID, JStaticFieldID, JStaticMethodID};

/// `getMainLooper().isCurrentThread()` — the `MainThreadMarker` proof
/// `RenderContext` carries. `!Send` and `!Sync` by construction: created
/// once per render and passed by borrow, never moved across threads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MainThread {
    _sealed: PhantomData<*const ()>,
}

impl MainThread {
    /// Answers the proof, or `None` off the main looper — the same contract
    /// `MainThreadMarker::new` states as an `Option`.
    pub(crate) fn new() -> Option<Self> {
        with_env(|env| {
            globals()
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
pub(crate) fn with_env<T>(body: impl FnOnce(&mut Env) -> T) -> T {
    JavaVM::singleton()
        .expect("JavaVM singleton is initialized by the first native call")
        .attach_current_thread(|env| -> jni::errors::Result<T> { Ok(body(env)) })
        .expect("attaching the current thread failed")
}

/// The frozen identifier table.
#[derive(Debug)]
#[allow(dead_code, reason = "some class refs only matter at resolve time")]
pub(crate) struct Bindings {
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

    // android/widget/FrameLayout — the button chrome's wrapper.
    frame_layout: Global<JClass<'static>>,
    frame_layout_ctor: JMethodID,

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

    // android/content/res/Configuration.
    configuration_ui_mode: JFieldID,

    // android/util/DisplayMetrics.
    display_metrics_density: JFieldID,
    display_metrics_scaled_density: JFieldID,

    // android/util/TypedValue.
    typed_value: Global<JClass<'static>>,
    typed_value_ctor: JMethodID,
    typed_value_type: JFieldID,
    typed_value_data: JFieldID,
    typed_value_resource_id: JFieldID,

    // android/os/Looper.
    looper: Global<JClass<'static>>,
    looper_get_main_looper: JStaticMethodID,
    looper_is_current_thread: JMethodID,

    // java/util/Locale — the system locale tag.
    locale: Global<JClass<'static>>,
    locale_get_default: JStaticMethodID,
    locale_to_language_tag: JMethodID,

    // android/view/View$MeasureSpec.
    measure_spec: Global<JClass<'static>>,
    measure_spec_make: JStaticMethodID,
    measure_spec_get_mode: JStaticMethodID,
    measure_spec_get_size: JStaticMethodID,
}

/// The resolved ids plus the host `Context`, installed once per process.
#[derive(Debug)]
pub(crate) struct Globals {
    bindings: Bindings,
    /// The host activity the backend constructs views against.
    context: Global<JObject<'static>>,
}

static GLOBALS: OnceLock<Globals> = OnceLock::new();

/// The density cache — `DisplayMetrics.density` and `scaledDensity` as f32
/// bits, refreshed on startup and on every configuration change, so the
/// measure path never crosses JNI to convert units.
static DENSITY: AtomicU32 = AtomicU32::new(0);
static SCALED_DENSITY: AtomicU32 = AtomicU32::new(0);

/// Another global reference to `view` — the way a leaf shares its platform
/// object with the watcher closures and layout face that outlive it.
pub(crate) fn retain(view: &Global<JObject<'static>>) -> Global<JObject<'static>> {
    with_env(|env| {
        env.new_global_ref(view.as_ref())
            .expect("a global reference to a live object always allocates")
    })
}

/// Refreshes the unit cache from `DisplayMetrics`.
pub(crate) fn refresh_metrics(env: &mut Env) -> jni::errors::Result<()> {
    let (density, scaled) = globals().bindings().display_metrics(env)?;
    DENSITY.store(density.to_bits(), Ordering::Relaxed);
    SCALED_DENSITY.store(scaled.to_bits(), Ordering::Relaxed);
    Ok(())
}

/// `DisplayMetrics.density` — px per dp.
pub(crate) fn density() -> f32 {
    f32::from_bits(DENSITY.load(Ordering::Relaxed))
}

/// `DisplayMetrics.scaledDensity` — px per sp.
pub(crate) fn scaled_density() -> f32 {
    f32::from_bits(SCALED_DENSITY.load(Ordering::Relaxed))
}

/// dp → px, rounding to whole pixels the way `View` frames expect.
pub(crate) fn dp_to_px(dp: f32) -> i32 {
    (dp * density()).round() as i32
}

/// px → dp.
pub(crate) fn px_to_dp(px: i32) -> f32 {
    px as f32 / density()
}

/// The installed globals.
///
/// # Panics
///
/// Before `nativeCreate` runs — every call site is inside the runtime's
/// lifetime, so an uninstalled table is unreachable.
pub(crate) fn globals() -> &'static Globals {
    GLOBALS.get().expect("JNI bindings are installed at nativeCreate")
}

impl Globals {
    /// The identifier table.
    pub(crate) const fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    /// The host activity — the `Context` every view constructor takes.
    pub(crate) fn context(&self) -> &Global<JObject<'static>> {
        &self.context
    }

    /// `context.getResources()`.
    pub(crate) fn resources(&self, env: &mut Env) -> jni::errors::Result<JObject<'static>> {
        // SAFETY: resolved id; `context` is an Activity (a Context).
        let resources = unsafe {
            env.call_method_unchecked(
                self.context(),
                self.bindings.context_get_resources,
                ReturnType::Object,
                &[],
            )?
        };
        resources.l()
    }

    /// `context.getTheme()`.
    pub(crate) fn theme(&self, env: &mut Env) -> jni::errors::Result<JObject<'static>> {
        // SAFETY: resolved id; `context` is an Activity (a Context).
        let theme = unsafe {
            env.call_method_unchecked(
                self.context(),
                self.bindings.context_get_theme,
                ReturnType::Object,
                &[],
            )?
        };
        theme.l()
    }
}

/// Resolves every identifier once and installs the [`Globals`]. Called from
/// `nativeCreate`'s frame — the only place the app classloader is guaranteed
/// to resolve `dev.waterui.android.*`.
///
/// # Panics
///
/// A second `nativeCreate` — the skeleton owns one host activity per process.
pub(crate) fn install(env: &mut Env, activity: &JObject) -> jni::errors::Result<()> {
    let globals = Globals {
        bindings: Bindings::resolve(env)?,
        context: env.new_global_ref(activity)?,
    };
    if GLOBALS.set(globals).is_err() {
        panic!("waterui-android hosts a single activity per process");
    }
    Ok(())
}

impl Bindings {
    /// The one-shot resolution — `findClass` for every class the backend
    /// touches, `GetMethodID`/`GetFieldID` for every member it calls.
    #[allow(
        clippy::too_many_lines,
        reason = "a flat id table is meant to be long"
    )]
    fn resolve(env: &mut Env) -> jni::errors::Result<Self> {
        let class = |name: &str| env.new_global_ref(env.find_class(name)?);

        let rust_view_group = class("dev/waterui/android/RustViewGroup")?;
        let rust_click_listener = class("dev/waterui/android/RustOnClickListener")?;
        let view = class("android/view/View")?;
        let view_group = class("android/view/ViewGroup")?;
        let text_view = class("android/widget/TextView")?;
        let space = class("android/widget/Space")?;
        let frame_layout = class("android/widget/FrameLayout")?;
        let button = class("android/widget/Button")?;
        let typeface = class("android/graphics/Typeface")?;
        let context = class("android/content/Context")?;
        let resources = class("android/content/res/Resources")?;
        let theme = class("android/content/res/Resources$Theme")?;
        let configuration = class("android/content/res/Configuration")?;
        let display_metrics = class("android/util/DisplayMetrics")?;
        let typed_value = class("android/util/TypedValue")?;
        let looper = class("android/os/Looper")?;
        let locale = class("java/util/Locale")?;
        let window_manager = class("android/view/WindowManager")?;
        let display = class("android/view/Display")?;
        let measure_spec = class("android/view/View$MeasureSpec")?;

        Ok(Self {
            rust_view_group_ctor: env.get_method_id(
                &rust_view_group,
                jni_str!("<init>"),
                jni_sig!((Landroid/content/Context;)V),
            )?,
            rust_view_group_set_handle: env.get_method_id(
                &rust_view_group,
                jni_str!("setHandle"),
                jni_sig!((J)V),
            )?,
            rust_view_group,
            rust_click_listener_ctor: env.get_method_id(
                &rust_click_listener,
                jni_str!("<init>"),
                jni_sig!((J)V),
            )?,
            rust_click_listener,

            view_measure: env.get_method_id(&view, jni_str!("measure"), jni_sig!((II)V))?,
            view_get_measured_width: env.get_method_id(
                &view,
                jni_str!("getMeasuredWidth"),
                jni_sig!(()I),
            )?,
            view_get_measured_height: env.get_method_id(
                &view,
                jni_str!("getMeasuredHeight"),
                jni_sig!(()I),
            )?,
            view_layout: env.get_method_id(&view, jni_str!("layout"), jni_sig!((IIII)V))?,
            view_request_layout: env
                .get_method_id(&view, jni_str!("requestLayout"), jni_sig!(()V))?,
            view_set_on_click_listener: env.get_method_id(
                &view,
                jni_str!("setOnClickListener"),
                jni_sig!((Landroid/view/View$OnClickListener;)V),
            )?,
            view_set_content_description: env.get_method_id(
                &view,
                jni_str!("setContentDescription"),
                jni_sig!((Ljava/lang/CharSequence;)V),
            )?,
            view_set_text_alignment: env.get_method_id(
                &view,
                jni_str!("setTextAlignment"),
                jni_sig!((I)V),
            )?,
            view_set_clickable: env
                .get_method_id(&view, jni_str!("setClickable"), jni_sig!((Z)V))?,
            view_set_important_for_accessibility: env.get_method_id(
                &view,
                jni_str!("setImportantForAccessibility"),
                jni_sig!((I)V),
            )?,
            view_set_background_color: env.get_method_id(
                &view,
                jni_str!("setBackgroundColor"),
                jni_sig!((I)V),
            )?,
            view_set_clip_children: env.get_method_id(
                &view_group,
                jni_str!("setClipChildren"),
                jni_sig!((Z)V),
            )?,

            view_group_add_view: env.get_method_id(
                &view_group,
                jni_str!("addView"),
                jni_sig!((Landroid/view/View;)V),
            )?,
            view_group_remove_view: env.get_method_id(
                &view_group,
                jni_str!("removeView"),
                jni_sig!((Landroid/view/View;)V),
            )?,

            text_view_ctor: env.get_method_id(
                &text_view,
                jni_str!("<init>"),
                jni_sig!((Landroid/content/Context;)V),
            )?,
            text_view_set_text: env.get_method_id(
                &text_view,
                jni_str!("setText"),
                jni_sig!((Ljava/lang/CharSequence;)V),
            )?,
            text_view_set_text_size: env.get_method_id(
                &text_view,
                jni_str!("setTextSize"),
                jni_sig!((F)V),
            )?,
            text_view_set_text_color: env.get_method_id(
                &text_view,
                jni_str!("setTextColor"),
                jni_sig!((I)V),
            )?,
            text_view_set_max_lines: env.get_method_id(
                &text_view,
                jni_str!("setMaxLines"),
                jni_sig!((I)V),
            )?,
            text_view_set_gravity: env.get_method_id(
                &text_view,
                jni_str!("setGravity"),
                jni_sig!((I)V),
            )?,
            text_view_set_typeface: env.get_method_id(
                &text_view,
                jni_str!("setTypeface"),
                jni_sig!((Landroid/graphics/Typeface;)V),
            )?,
            text_view,

            space_ctor: env.get_method_id(
                &space,
                jni_str!("<init>"),
                jni_sig!((Landroid/content/Context;)V),
            )?,
            space,

            frame_layout_ctor: env.get_method_id(
                &frame_layout,
                jni_str!("<init>"),
                jni_sig!((Landroid/content/Context;)V),
            )?,
            frame_layout,

            button_ctor: env.get_method_id(
                &button,
                jni_str!("<init>"),
                jni_sig!((Landroid/content/Context;)V),
            )?,
            button,

            typeface_default: env.get_static_field_id(
                &typeface,
                jni_str!("DEFAULT"),
                jni_sig!(Landroid/graphics/Typeface;),
            )?,
            typeface_default_bold: env.get_static_field_id(
                &typeface,
                jni_str!("DEFAULT_BOLD"),
                jni_sig!(Landroid/graphics/Typeface;),
            )?,
            typeface_monospace: env.get_static_field_id(
                &typeface,
                jni_str!("MONOSPACE"),
                jni_sig!(Landroid/graphics/Typeface;),
            )?,
            typeface,

            context_get_resources: env.get_method_id(
                &context,
                jni_str!("getResources"),
                jni_sig!(()Landroid/content/res/Resources;),
            )?,
            context_get_theme: env.get_method_id(
                &context,
                jni_str!("getTheme"),
                jni_sig!(()Landroid/content/res/Resources$Theme;),
            )?,
            context_get_system_service: env.get_method_id(
                &context,
                jni_str!("getSystemService"),
                jni_sig!((Ljava/lang/String;)Ljava/lang/Object;),
            )?,
            window_manager_get_default_display: env.get_method_id(
                &window_manager,
                jni_str!("getDefaultDisplay"),
                jni_sig!(()Landroid/view/Display;),
            )?,
            display_get_refresh_rate: env.get_method_id(
                &display,
                jni_str!("getRefreshRate"),
                jni_sig!(()F),
            )?,

            resources_get_display_metrics: env.get_method_id(
                &resources,
                jni_str!("getDisplayMetrics"),
                jni_sig!(()Landroid/util/DisplayMetrics;),
            )?,
            resources_get_configuration: env.get_method_id(
                &resources,
                jni_str!("getConfiguration"),
                jni_sig!(()Landroid/content/res/Configuration;),
            )?,
            resources_get_color: env.get_method_id(
                &resources,
                jni_str!("getColor"),
                jni_sig!((ILandroid/content/res/Resources$Theme;)I),
            )?,

            theme_resolve_attribute: env.get_method_id(
                &theme,
                jni_str!("resolveAttribute"),
                jni_sig!((ILandroid/util/TypedValue;Z)Z),
            )?,
            theme,

            configuration_ui_mode: env.get_field_id(
                &configuration,
                jni_str!("uiMode"),
                jni_sig!(I),
            )?,

            display_metrics_density: env.get_field_id(
                &display_metrics,
                jni_str!("density"),
                jni_sig!(F),
            )?,
            display_metrics_scaled_density: env.get_field_id(
                &display_metrics,
                jni_str!("scaledDensity"),
                jni_sig!(F),
            )?,

            typed_value_ctor: env.get_method_id(
                &typed_value,
                jni_str!("<init>"),
                jni_sig!(()V),
            )?,
            typed_value_type: env.get_field_id(&typed_value, jni_str!("type"), jni_sig!(I))?,
            typed_value_data: env.get_field_id(&typed_value, jni_str!("data"), jni_sig!(I))?,
            typed_value_resource_id: env.get_field_id(
                &typed_value,
                jni_str!("resourceId"),
                jni_sig!(I),
            )?,
            typed_value,

            looper_get_main_looper: env.get_static_method_id(
                &looper,
                jni_str!("getMainLooper"),
                jni_sig!(()Landroid/os/Looper;),
            )?,
            looper_is_current_thread: env.get_method_id(
                &looper,
                jni_str!("isCurrentThread"),
                jni_sig!(()Z),
            )?,
            looper,

            locale_get_default: env.get_static_method_id(
                &locale,
                jni_str!("getDefault"),
                jni_sig!(()Ljava/util/Locale;),
            )?,
            locale_to_language_tag: env.get_method_id(
                &locale,
                jni_str!("toLanguageTag"),
                jni_sig!(()Ljava/lang/String;),
            )?,
            locale,

            measure_spec_make: env.get_static_method_id(
                &measure_spec,
                jni_str!("makeMeasureSpec"),
                jni_sig!((II)I),
            )?,
            measure_spec_get_mode: env.get_static_method_id(
                &measure_spec,
                jni_str!("getMode"),
                jni_sig!((I)I),
            )?,
            measure_spec_get_size: env.get_static_method_id(
                &measure_spec,
                jni_str!("getSize"),
                jni_sig!((I)I),
            )?,
            measure_spec,
        })
    }
}

/// A `View` subclass' platform object, constructed against the host context
/// and returned as a [`Global`]: the leaf owns it, exactly like the retained
/// object an Apple leaf holds.
fn construct(
    env: &mut Env,
    class: &Global<JClass<'static>>,
    ctor: JMethodID,
) -> jni::errors::Result<Global<JObject<'static>>> {
    // SAFETY: `class`/`ctor` come from the resolved table, and the
    // constructor signature is the shared `(Context)` shape every `View`
    // subclass declares — the only argument is the host context.
    let object = unsafe {
        env.new_object_unchecked(class, ctor, &[jvalue {
            l: globals().context().as_raw(),
        }])?
    };
    env.new_global_ref(&object)
}

impl Bindings {
    /// `new RustViewGroup(context)`.
    pub(crate) fn new_rust_view_group(
        &self,
        env: &mut Env,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        construct(env, &self.rust_view_group, self.rust_view_group_ctor)
    }

    /// `new RustOnClickListener(handle)`.
    pub(crate) fn new_click_listener(
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

    /// `new Space(context)` — the empty leaf.
    pub(crate) fn new_space(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        construct(env, &self.space, self.space_ctor)
    }

    /// `new TextView(context)`.
    pub(crate) fn new_text_view(
        &self,
        env: &mut Env,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        construct(env, &self.text_view, self.text_view_ctor)
    }

    /// `new FrameLayout(context)`.
    pub(crate) fn new_frame_layout(
        &self,
        env: &mut Env,
    ) -> jni::errors::Result<Global<JObject<'static>>> {
        construct(env, &self.frame_layout, self.frame_layout_ctor)
    }

    /// `new Button(context)` — the chrome a button leaf fills.
    pub(crate) fn new_button(&self, env: &mut Env) -> jni::errors::Result<Global<JObject<'static>>> {
        construct(env, &self.button, self.button_ctor)
    }

    /// `group.setHandle(handle)` — the `ContainerState` the Kotlin bridge
    /// forwards `onMeasure`/`onLayout` to.
    pub(crate) fn set_handle(
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
                ReturnType::Void,
                &[jvalue { j: handle }],
            )?;
        }
        Ok(())
    }

    /// `view.measure(widthSpec, heightSpec)` — a platform measure against
    /// explicit specs, the probe every `ViewSubView` runs.
    pub(crate) fn measure(
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
                ReturnType::Void,
                &[jvalue { i: width_spec }, jvalue { i: height_spec }],
            )?;
        }
        Ok(())
    }

    /// `(view.getMeasuredWidth(), view.getMeasuredHeight())` in pixels.
    pub(crate) fn measured_size(
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
    pub(crate) fn layout(
        &self,
        env: &mut Env,
        view: &JObject,
        left: jint,
        top: jint,
        right: jint,
        bottom: jint,
    ) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(
                view,
                self.view_layout,
                ReturnType::Void,
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
    pub(crate) fn request_layout(&self, env: &mut Env, view: &JObject) -> jni::errors::Result<()> {
        // SAFETY: resolved id; `view` is a View.
        unsafe {
            env.call_method_unchecked(view, self.view_request_layout, ReturnType::Void, &[])?;
        }
        Ok(())
    }

    /// `parent.addView(child)` — `NativeLeaf::mount`.
    pub(crate) fn add_view(
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
                ReturnType::Void,
                &[jvalue { l: child.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `parent.removeView(child)` — `Mounted`'s detach.
    pub(crate) fn remove_view(
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
                ReturnType::Void,
                &[jvalue { l: child.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `view.setOnClickListener(listener)` — `listener` may be null to clear.
    pub(crate) fn set_on_click_listener(
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
                ReturnType::Void,
                &[jvalue { l: raw }],
            )?;
        }
        Ok(())
    }

    /// `view.setContentDescription(text)` — the spoken label.
    pub(crate) fn set_content_description(
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
                ReturnType::Void,
                &[jvalue { l: text.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `view.setBackgroundColor(argb)` — packed ARGB, for the window's
    /// resolved background behind the mounted content.
    pub(crate) fn set_background_color(
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
                ReturnType::Void,
                &[jvalue { i: argb }],
            )?;
        }
        Ok(())
    }

    /// `viewGroup.setClipChildren(clip)` — off by default so a child may
    /// draw past its bounds the way Flutter lets it.
    pub(crate) fn set_clip_children(
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
                ReturnType::Void,
                &[jvalue { z: i8::from(clip) }],
            )?;
        }
        Ok(())
    }

    /// `textView.setText(text)`.
    pub(crate) fn set_text(
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
                ReturnType::Void,
                &[jvalue { l: text.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `textView.setTextSize(size)` — the single-float overload, which
    /// already reads the argument as scale-independent pixels.
    pub(crate) fn set_text_size(
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
                ReturnType::Void,
                &[jvalue { f: size_sp }],
            )?;
        }
        Ok(())
    }

    /// `textView.setTextColor(argb)` — packed ARGB.
    pub(crate) fn set_text_color(
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
                ReturnType::Void,
                &[jvalue { i: argb }],
            )?;
        }
        Ok(())
    }

    /// `textView.setMaxLines(lines)`.
    pub(crate) fn set_max_lines(
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
                ReturnType::Void,
                &[jvalue { i: lines }],
            )?;
        }
        Ok(())
    }

    /// `textView.setGravity(gravity)` — `android.view.Gravity` flags.
    pub(crate) fn set_gravity(
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
                ReturnType::Void,
                &[jvalue { i: gravity }],
            )?;
        }
        Ok(())
    }

    /// `textView.setTypeface(typeface)`.
    pub(crate) fn set_typeface(
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
                ReturnType::Void,
                &[jvalue { l: typeface.as_raw() }],
            )?;
        }
        Ok(())
    }

    /// `view.setTextAlignment(alignment)` — `View.TEXT_ALIGNMENT_*`.
    pub(crate) fn set_text_alignment(
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
                ReturnType::Void,
                &[jvalue { i: alignment }],
            )?;
        }
        Ok(())
    }

    /// `view.setClickable(clickable)` — a label drawn above chrome must not
    /// swallow the press it decorates.
    pub(crate) fn set_clickable(
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
                ReturnType::Void,
                &[jvalue { z: i8::from(clickable) }],
            )?;
        }
        Ok(())
    }

    /// `view.setImportantForAccessibility(mode)` —
    /// `View.IMPORTANT_FOR_ACCESSIBILITY_*`.
    pub(crate) fn set_important_for_accessibility(
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
                ReturnType::Void,
                &[jvalue { i: mode }],
            )?;
        }
        Ok(())
    }

    /// `Typeface` static field read (`DEFAULT`, `DEFAULT_BOLD`, `MONOSPACE`).
    pub(crate) fn typeface(
        &self,
        env: &mut Env,
        face: Face,
    ) -> jni::errors::Result<JObject<'static>> {
        let field = match face {
            Face::Default => self.typeface_default,
            Face::DefaultBold => self.typeface_default_bold,
            Face::Monospace => self.typeface_monospace,
        };
        // SAFETY: resolved static field on the resolved class.
        let value =
            unsafe { env.get_static_field_unchecked(&self.typeface, field, JavaType::Object)? };
        value.l()
    }

    /// `Looper.getMainLooper().isCurrentThread()` — the `MainThread` proof.
    pub(crate) fn is_current_thread(&self, env: &mut Env) -> jni::errors::Result<bool> {
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

    /// `resources.getDisplayMetrics()` → `(density, scaledDensity)`.
    pub(crate) fn display_metrics(&self, env: &mut Env) -> jni::errors::Result<(f32, f32)> {
        let resources = globals().resources(env)?;
        // SAFETY: resolved id; `resources` is a Resources.
        let metrics = unsafe {
            env.call_method_unchecked(
                &resources,
                self.resources_get_display_metrics,
                ReturnType::Object,
                &[],
            )?
        };
        let metrics = metrics.l()?;
        // SAFETY: resolved fields on DisplayMetrics.
        let (density, scaled) = unsafe {
            (
                env.get_field_unchecked(
                    &metrics,
                    self.display_metrics_density,
                    JavaType::Primitive(Primitive::Float),
                )?,
                env.get_field_unchecked(
                    &metrics,
                    self.display_metrics_scaled_density,
                    JavaType::Primitive(Primitive::Float),
                )?,
            )
        };
        Ok((density.f()?, scaled.f()?))
    }

    /// `resources.getConfiguration().uiMode` — raw `Configuration.uiMode`.
    pub(crate) fn ui_mode(&self, env: &mut Env) -> jni::errors::Result<jint> {
        let resources = globals().resources(env)?;
        // SAFETY: resolved id; `resources` is a Resources.
        let configuration = unsafe {
            env.call_method_unchecked(
                &resources,
                self.resources_get_configuration,
                ReturnType::Object,
                &[],
            )?
        };
        let configuration = configuration.l()?;
        // SAFETY: resolved field on Configuration.
        let ui_mode = unsafe {
            env.get_field_unchecked(
                &configuration,
                self.configuration_ui_mode,
                JavaType::Primitive(Primitive::Int),
            )?
        };
        ui_mode.i()
    }

    /// `theme.resolveAttribute(attr, typedValue, true)` +
    /// `resources.getColor(resourceId, theme)` — a themed color as ARGB.
    ///
    /// Answers `None` when the theme does not resolve the attribute to a
    /// color; a missing token is the caller's to fill.
    pub(crate) fn theme_color(
        &self,
        env: &mut Env,
        attr: jint,
    ) -> jni::errors::Result<Option<jint>> {
        let theme = globals().theme(env)?;
        let resources = globals().resources(env)?;
        // SAFETY: resolved constructor; `value` is a fresh TypedValue the
        // resolve call fills by contract.
        let value =
            unsafe { env.new_object_unchecked(&self.typed_value, self.typed_value_ctor, &[])? };
        // SAFETY: resolved method; `value` is a TypedValue.
        let resolved = unsafe {
            env.call_method_unchecked(
                &theme,
                self.theme_resolve_attribute,
                ReturnType::Primitive(Primitive::Boolean),
                &[
                    jvalue { i: attr },
                    jvalue { l: value.as_raw() },
                    jvalue { z: 1 },
                ],
            )?
        };
        if !resolved.z()? {
            return Ok(None);
        }
        // SAFETY: resolved fields on the TypedValue `resolveAttribute` filled.
        let (kind, data, resource_id) = unsafe {
            (
                env.get_field_unchecked(
                    &value,
                    self.typed_value_type,
                    JavaType::Primitive(Primitive::Int),
                )?,
                env.get_field_unchecked(
                    &value,
                    self.typed_value_data,
                    JavaType::Primitive(Primitive::Int),
                )?,
                env.get_field_unchecked(
                    &value,
                    self.typed_value_resource_id,
                    JavaType::Primitive(Primitive::Int),
                )?,
            )
        };
        let kind = kind.i()?;
        // `TYPE_INT_COLOR_*` (16..=31) carries the ARGB in `data`; a
        // `TYPE_REFERENCE` (1) names a color resource to resolve.
        if (16..32).contains(&kind) {
            return Ok(Some(data.i()?));
        }
        if kind != 1 {
            return Ok(None);
        }
        // SAFETY: resolved method; `resource_id` names a color resource.
        let color = unsafe {
            env.call_method_unchecked(
                &resources,
                self.resources_get_color,
                ReturnType::Primitive(Primitive::Int),
                &[jvalue { i: resource_id.i()? }, jvalue { l: theme.as_raw() }],
            )?
        };
        Ok(Some(color.i()?))
    }

    /// `context.getSystemService("window").getDefaultDisplay()
    /// .getRefreshRate()` — the display's nominal refresh in Hz, for the
    /// executor's frame budget; `None` when the platform reports none.
    pub(crate) fn refresh_rate_hz(&self, env: &mut Env) -> jni::errors::Result<Option<f32>> {
        let name = env.new_string("window")?;
        // SAFETY: resolved method; `context` is an Activity.
        let service = unsafe {
            env.call_method_unchecked(
                globals().context(),
                self.context_get_system_service,
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
                self.window_manager_get_default_display,
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
                self.display_get_refresh_rate,
                ReturnType::Primitive(Primitive::Float),
                &[],
            )?
        };
        let hz = rate.f()?;
        // `getRefreshRate` reports 0 when no rate is known; a non-positive
        // answer is "unavailable", not a budget.
        Ok((hz > 0.0).then_some(hz))
    }

    /// `Locale.getDefault().toLanguageTag()` — the platform's preferred
    /// locale as a BCP 47 tag.
    pub(crate) fn locale_tag(&self, env: &mut Env) -> jni::errors::Result<String> {
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
        let tag = JString::from(tag.l()?);
        tag.try_to_string(env)
    }

    /// `View.MeasureSpec.makeMeasureSpec(size, mode)` — the packed spec.
    pub(crate) fn make_measure_spec(
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
    pub(crate) fn measure_spec_mode(&self, env: &mut Env, spec: jint) -> jni::errors::Result<jint> {
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
    pub(crate) fn measure_spec_size(&self, env: &mut Env, spec: jint) -> jni::errors::Result<jint> {
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
}

/// The `Typeface` faces the text port picks between — a semantic request,
/// not a family name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Face {
    /// The platform's default proportional face.
    Default,
    /// The platform's bold face for `FontWeight::SemiBold` and up.
    DefaultBold,
    /// The platform's fixed-pitch face for `FontDesign::Monospaced`.
    Monospace,
}
