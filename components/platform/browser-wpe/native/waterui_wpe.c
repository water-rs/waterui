#include "waterui_wpe.h"

#include <dlfcn.h>
#include <drm_fourcc.h>
#include <fcntl.h>
#include <gio/gio.h>
#include <glib-object.h>
#include <glib.h>
#include <jsc/jsc.h>
#include <json-glib/json-glib.h>
#include <libsoup/soup.h>
#include <string.h>
#include <unistd.h>
#include <wpe/headless/wpe-headless.h>
#include <wpe/webkit.h>
#include <wpe/wpe-platform.h>

enum {
    WATER_WPE_EVENT_NAVIGATION_STARTED = 1,
    WATER_WPE_EVENT_LOADING = 2,
    WATER_WPE_EVENT_LOADED = 3,
    WATER_WPE_EVENT_REDIRECT = 4,
    WATER_WPE_EVENT_LOAD_FAILED = 5,
    WATER_WPE_EVENT_TLS_FAILED = 6,
    WATER_WPE_EVENT_HISTORY_CHANGED = 7,
};

struct WaterWpeRuntime {
    GMainContext *context;
    WPEDisplay *delegate;
    WPEDisplay *display;
};

struct WaterWpePage {
    WaterWpeRuntime *runtime;
    WebKitWebView *web_view;
    WebKitUserContentManager *content_manager;
    WPEView *view;
    WPEToplevel *toplevel;
    WaterWpeEventCallback event_callback;
    WaterWpeFrameCallback frame_callback;
    WaterWpeMessageCallback message_callback;
    void *user_data;
    WaterWpeDestroyNotify destroy_user_data;
    gboolean redirects_enabled;
    char *last_uri;
};

typedef struct {
    GMainContext *context;
    WPEView *view;
    WPEBuffer *buffer;
    gint references;
} WaterWpeFrameToken;

typedef struct _WaterView {
    WPEView parent_instance;
    WaterWpePage *page;
} WaterView;

typedef struct _WaterViewClass {
    WPEViewClass parent_class;
} WaterViewClass;

typedef struct _WaterToplevel {
    WPEToplevel parent_instance;
} WaterToplevel;

typedef struct _WaterToplevelClass {
    WPEToplevelClass parent_class;
} WaterToplevelClass;

typedef struct _WaterDisplay {
    WPEDisplay parent_instance;
    WPEDisplay *delegate;
    WaterWpeRuntime *runtime;
    WPEBufferFormats *formats;
} WaterDisplay;

typedef struct _WaterDisplayClass {
    WPEDisplayClass parent_class;
} WaterDisplayClass;

#define WATER_TYPE_VIEW (water_view_get_type())
#define WATER_IS_VIEW(instance) \
    (G_TYPE_CHECK_INSTANCE_TYPE((instance), WATER_TYPE_VIEW))

G_DEFINE_TYPE(WaterView, water_view, WPE_TYPE_VIEW)
G_DEFINE_TYPE(WaterToplevel, water_toplevel, WPE_TYPE_TOPLEVEL)
G_DEFINE_TYPE(WaterDisplay, water_display, WPE_TYPE_DISPLAY)

static WaterWpeFrameToken *water_wpe_frame_token_ref(WaterWpeFrameToken *token)
{
    g_atomic_int_inc(&token->references);
    return token;
}

static void water_wpe_frame_token_unref(WaterWpeFrameToken *token)
{
    if (!g_atomic_int_dec_and_test(&token->references))
        return;
    g_object_unref(token->buffer);
    g_object_unref(token->view);
    g_main_context_unref(token->context);
    g_free(token);
}

static gboolean water_wpe_frame_presented_on_main(gpointer user_data)
{
    WaterWpeFrameToken *token = user_data;
    wpe_view_buffer_rendered(token->view, token->buffer);
    water_wpe_frame_token_unref(token);
    return G_SOURCE_REMOVE;
}

typedef struct {
    WaterWpeFrameToken *token;
    int release_fence_fd;
} WaterWpeRelease;

static gboolean water_wpe_frame_released_on_main(gpointer user_data)
{
    WaterWpeRelease *release = user_data;
    if (release->release_fence_fd >= 0)
        wpe_buffer_set_release_fence(release->token->buffer, release->release_fence_fd);
    wpe_view_buffer_released(release->token->view, release->token->buffer);
    water_wpe_frame_token_unref(release->token);
    g_free(release);
    return G_SOURCE_REMOVE;
}

static void water_view_toplevel_changed(WPEView *view)
{
    WPEToplevel *toplevel = wpe_view_get_toplevel(view);
    if (!toplevel) {
        wpe_view_unmap(view);
        return;
    }
    int width = 0;
    int height = 0;
    wpe_toplevel_get_size(toplevel, &width, &height);
    if (width > 0 && height > 0)
        wpe_view_resized(view, width, height);
    wpe_view_map(view);
}

static void water_view_constructed(GObject *object)
{
    G_OBJECT_CLASS(water_view_parent_class)->constructed(object);
    g_signal_connect_swapped(
        object,
        "notify::toplevel",
        G_CALLBACK(water_view_toplevel_changed),
        object);
}

static gboolean water_view_render_buffer(
    WPEView *view,
    WPEBuffer *buffer,
    const WPERectangle *damage_rects,
    guint n_damage_rects,
    GError **error)
{
    (void)damage_rects;
    (void)n_damage_rects;
    (void)error;

    WaterWpePage *page = ((WaterView *)view)->page;
    g_assert(page != NULL);
    g_assert(WPE_IS_BUFFER_DMA_BUF(buffer));

    WPEBufferDMABuf *dma_buf = WPE_BUFFER_DMA_BUF(buffer);
    guint32 n_planes = wpe_buffer_dma_buf_get_n_planes(dma_buf);
    g_assert_cmpuint(n_planes, >, 0);
    g_assert_cmpuint(n_planes, <=, WATER_WPE_MAX_PLANES);
    g_assert_cmpuint(n_planes, ==, 1);
    g_assert_true(
        wpe_buffer_dma_buf_get_modifier(dma_buf) == DRM_FORMAT_MOD_LINEAR);

    WaterWpeFrameToken *token = g_new0(WaterWpeFrameToken, 1);
    token->context = g_main_context_ref(page->runtime->context);
    token->view = g_object_ref(view);
    token->buffer = g_object_ref(buffer);
    token->references = 1;

    WaterWpeFrame frame = {
        .token = token,
        .width = (uint32_t)wpe_buffer_get_width(buffer),
        .height = (uint32_t)wpe_buffer_get_height(buffer),
        .format = wpe_buffer_dma_buf_get_format(dma_buf),
        .modifier = wpe_buffer_dma_buf_get_modifier(dma_buf),
        .n_planes = n_planes,
        .fds = { -1, -1, -1, -1 },
        .rendering_fence_fd = wpe_buffer_take_rendering_fence(buffer),
    };
    for (guint32 plane = 0; plane < n_planes; ++plane) {
        int source_fd = wpe_buffer_dma_buf_get_fd(dma_buf, plane);
        frame.fds[plane] = fcntl(source_fd, F_DUPFD_CLOEXEC, 0);
        g_assert_cmpint(frame.fds[plane], >=, 0);
        frame.offsets[plane] = wpe_buffer_dma_buf_get_offset(dma_buf, plane);
        frame.strides[plane] = wpe_buffer_dma_buf_get_stride(dma_buf, plane);
    }
    page->frame_callback(page->user_data, &frame);
    return TRUE;
}

static void water_view_class_init(WaterViewClass *klass)
{
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->constructed = water_view_constructed;
    WPEViewClass *view_class = WPE_VIEW_CLASS(klass);
    view_class->render_buffer = water_view_render_buffer;
}

static void water_view_init(WaterView *view)
{
    view->page = NULL;
}

static void water_toplevel_constructed(GObject *object)
{
    G_OBJECT_CLASS(water_toplevel_parent_class)->constructed(object);
    wpe_toplevel_state_changed(
        WPE_TOPLEVEL(object),
        WPE_TOPLEVEL_STATE_ACTIVE);
}

static gboolean water_toplevel_resize(
    WPEToplevel *toplevel,
    int width,
    int height)
{
    wpe_toplevel_resized(toplevel, width, height);
    return TRUE;
}

static void water_toplevel_class_init(WaterToplevelClass *klass)
{
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->constructed = water_toplevel_constructed;
    WPEToplevelClass *toplevel_class = WPE_TOPLEVEL_CLASS(klass);
    toplevel_class->resize = water_toplevel_resize;
}

static void water_toplevel_init(WaterToplevel *toplevel)
{
    (void)toplevel;
}

static gboolean water_display_connect(WPEDisplay *display, GError **error)
{
    (void)display;
    (void)error;
    return TRUE;
}

static WPEView *water_display_create_view(WPEDisplay *display)
{
    return WPE_VIEW(g_object_new(water_view_get_type(), "display", display, NULL));
}

static WPEToplevel *water_display_create_toplevel(
    WPEDisplay *display,
    guint max_views)
{
    return WPE_TOPLEVEL(g_object_new(
        water_toplevel_get_type(),
        "display",
        display,
        "max-views",
        max_views,
        NULL));
}

static gpointer water_display_get_egl_display(
    WPEDisplay *display,
    GError **error)
{
    WaterDisplay *water_display = (WaterDisplay *)display;
    return wpe_display_get_egl_display(water_display->delegate, error);
}

static WPEDRMDevice *water_display_get_drm_device(WPEDisplay *display)
{
    WaterDisplay *water_display = (WaterDisplay *)display;
    return wpe_display_get_drm_device(water_display->delegate);
}

static WPEBufferFormats *water_display_get_preferred_buffer_formats(
    WPEDisplay *display)
{
    return ((WaterDisplay *)display)->formats;
}

static gboolean water_display_use_explicit_sync(WPEDisplay *display)
{
    (void)display;
    return TRUE;
}

static void water_display_dispose(GObject *object)
{
    WaterDisplay *display = (WaterDisplay *)object;
    g_clear_object(&display->formats);
    g_clear_object(&display->delegate);
    G_OBJECT_CLASS(water_display_parent_class)->dispose(object);
}

static void water_display_class_init(WaterDisplayClass *klass)
{
    GObjectClass *object_class = G_OBJECT_CLASS(klass);
    object_class->dispose = water_display_dispose;
    WPEDisplayClass *display_class = WPE_DISPLAY_CLASS(klass);
    display_class->connect = water_display_connect;
    display_class->create_view = water_display_create_view;
    display_class->create_toplevel = water_display_create_toplevel;
    display_class->get_egl_display = water_display_get_egl_display;
    display_class->get_drm_device = water_display_get_drm_device;
    display_class->get_preferred_buffer_formats =
        water_display_get_preferred_buffer_formats;
    display_class->use_explicit_sync = water_display_use_explicit_sync;
}

static void water_display_init(WaterDisplay *display)
{
    display->delegate = NULL;
    display->runtime = NULL;
    display->formats = NULL;
}

static char *water_wpe_runtime_root(void)
{
    Dl_info info;
    g_assert(dladdr((const void *)&water_wpe_abi_version, &info) != 0);
    g_assert(info.dli_fname != NULL);
    char *library = g_canonicalize_filename(info.dli_fname, NULL);
    char *lib = g_path_get_dirname(library);
    char *root = g_path_get_dirname(lib);
    g_free(lib);
    g_free(library);
    return root;
}

static void water_wpe_configure_runtime_paths(void)
{
    char *root = water_wpe_runtime_root();
    char *exec_path = g_build_filename(root, "libexec", "wpe-webkit-2.0", NULL);
    char *bundle_path =
        g_build_filename(root, "lib", "wpe-webkit-2.0", "injected-bundle", NULL);
    char *data_path = g_build_filename(root, "share", NULL);
    char *plugin_path = g_build_filename(root, "lib", "gstreamer-1.0", NULL);
    char *plugin_scanner =
        g_build_filename(root, "libexec", "gstreamer-1.0", "gst-plugin-scanner", NULL);
    char *gio_modules = g_build_filename(root, "lib", "gio", "modules", NULL);
    g_setenv("WEBKIT_EXEC_PATH", exec_path, TRUE);
    g_setenv("WEBKIT_INJECTED_BUNDLE_PATH", bundle_path, TRUE);
    g_setenv("GST_PLUGIN_SYSTEM_PATH_1_0", plugin_path, TRUE);
    g_setenv("GST_PLUGIN_SCANNER_1_0", plugin_scanner, TRUE);
    g_setenv("GIO_EXTRA_MODULES", gio_modules, TRUE);
    const char *system_data_path = g_getenv("XDG_DATA_DIRS");
    char *combined_data_path = system_data_path == NULL
        ? g_strdup(data_path)
        : g_strconcat(data_path, G_SEARCHPATH_SEPARATOR_S, system_data_path, NULL);
    g_setenv("XDG_DATA_DIRS", combined_data_path, TRUE);
    g_free(combined_data_path);
    g_free(gio_modules);
    g_free(plugin_scanner);
    g_free(plugin_path);
    g_free(data_path);
    g_free(bundle_path);
    g_free(exec_path);
    g_free(root);
}

uint32_t water_wpe_abi_version(void)
{
    return WATER_WPE_ABI_VERSION;
}

WaterWpeRuntime *water_wpe_runtime_new(char **error)
{
    g_assert(error != NULL);
    *error = NULL;
    if (webkit_get_major_version() != WEBKIT_MAJOR_VERSION ||
        webkit_get_minor_version() != WEBKIT_MINOR_VERSION ||
        webkit_get_micro_version() != WEBKIT_MICRO_VERSION) {
        *error = g_strdup_printf(
            "WPE WebKit runtime version %u.%u.%u does not match bridge version %u.%u.%u",
            webkit_get_major_version(),
            webkit_get_minor_version(),
            webkit_get_micro_version(),
            WEBKIT_MAJOR_VERSION,
            WEBKIT_MINOR_VERSION,
            WEBKIT_MICRO_VERSION);
        return NULL;
    }
    water_wpe_configure_runtime_paths();

    WaterWpeRuntime *runtime = g_new0(WaterWpeRuntime, 1);
    runtime->context = g_main_context_ref_thread_default();
    runtime->delegate = wpe_display_headless_new();
    GError *display_error = NULL;
    if (!wpe_display_connect(runtime->delegate, &display_error)) {
        *error = g_strdup(display_error->message);
        g_error_free(display_error);
        g_object_unref(runtime->delegate);
        g_main_context_unref(runtime->context);
        g_free(runtime);
        return NULL;
    }

    WaterDisplay *display =
        (WaterDisplay *)g_object_new(water_display_get_type(), NULL);
    display->runtime = runtime;
    display->delegate = g_object_ref(runtime->delegate);
    WPEDRMDevice *device = wpe_display_get_drm_device(runtime->delegate);
    WPEBufferFormatsBuilder *builder =
        wpe_buffer_formats_builder_new(device);
    wpe_buffer_formats_builder_append_group(
        builder,
        device,
        WPE_BUFFER_FORMAT_USAGE_RENDERING);
    wpe_buffer_formats_builder_append_format(
        builder,
        DRM_FORMAT_ARGB8888,
        DRM_FORMAT_MOD_LINEAR);
    wpe_buffer_formats_builder_append_format(
        builder,
        DRM_FORMAT_XRGB8888,
        DRM_FORMAT_MOD_LINEAR);
    display->formats = wpe_buffer_formats_builder_end(builder);
    wpe_buffer_formats_builder_unref(builder);
    runtime->display = WPE_DISPLAY(display);
    return runtime;
}

void water_wpe_runtime_free(WaterWpeRuntime *runtime)
{
    g_assert(runtime != NULL);
    while (g_main_context_iteration(runtime->context, FALSE)) {
    }
    g_object_unref(runtime->display);
    g_object_unref(runtime->delegate);
    g_main_context_unref(runtime->context);
    g_free(runtime);
}

bool water_wpe_runtime_iteration(WaterWpeRuntime *runtime)
{
    g_assert(runtime != NULL);
    return g_main_context_iteration(runtime->context, FALSE);
}

void water_wpe_string_free(char *string)
{
    g_free(string);
}

static void water_wpe_emit_history(WaterWpePage *page)
{
    guint state = 0;
    if (webkit_web_view_can_go_back(page->web_view))
        state |= 1;
    if (webkit_web_view_can_go_forward(page->web_view))
        state |= 2;
    page->event_callback(
        page->user_data,
        WATER_WPE_EVENT_HISTORY_CHANGED,
        "",
        "",
        state);
}

static void water_wpe_load_changed(
    WebKitWebView *web_view,
    WebKitLoadEvent event,
    WaterWpePage *page)
{
    const char *uri = webkit_web_view_get_uri(web_view);
    if (!uri)
        uri = "about:blank";
    switch (event) {
    case WEBKIT_LOAD_STARTED:
        g_free(page->last_uri);
        page->last_uri = g_strdup(uri);
        page->event_callback(
            page->user_data,
            WATER_WPE_EVENT_NAVIGATION_STARTED,
            uri,
            "",
            0);
        break;
    case WEBKIT_LOAD_REDIRECTED:
        page->event_callback(
            page->user_data,
            WATER_WPE_EVENT_REDIRECT,
            page->last_uri ? page->last_uri : uri,
            uri,
            0);
        g_free(page->last_uri);
        page->last_uri = g_strdup(uri);
        break;
    case WEBKIT_LOAD_COMMITTED:
        break;
    case WEBKIT_LOAD_FINISHED:
        page->event_callback(
            page->user_data,
            WATER_WPE_EVENT_LOADED,
            "",
            "",
            0);
        water_wpe_emit_history(page);
        break;
    }
}

static void water_wpe_progress_changed(
    WebKitWebView *web_view,
    GParamSpec *spec,
    WaterWpePage *page)
{
    (void)spec;
    page->event_callback(
        page->user_data,
        WATER_WPE_EVENT_LOADING,
        "",
        "",
        webkit_web_view_get_estimated_load_progress(web_view));
}

static void water_wpe_history_changed(
    WebKitWebView *web_view,
    GParamSpec *spec,
    WaterWpePage *page)
{
    (void)web_view;
    (void)spec;
    water_wpe_emit_history(page);
}

static gboolean water_wpe_decide_policy(
    WebKitWebView *web_view,
    WebKitPolicyDecision *decision,
    WebKitPolicyDecisionType type,
    WaterWpePage *page)
{
    (void)web_view;
    if (type != WEBKIT_POLICY_DECISION_TYPE_NAVIGATION_ACTION)
        return FALSE;
    WebKitNavigationAction *action =
        webkit_navigation_policy_decision_get_navigation_action(
            WEBKIT_NAVIGATION_POLICY_DECISION(decision));
    if (webkit_navigation_action_is_redirect(action) &&
        !page->redirects_enabled) {
        webkit_policy_decision_ignore(decision);
        return TRUE;
    }
    return FALSE;
}

static gboolean water_wpe_load_failed(
    WebKitWebView *web_view,
    WebKitLoadEvent event,
    const char *failing_uri,
    GError *error,
    WaterWpePage *page)
{
    (void)web_view;
    (void)event;
    (void)failing_uri;
    page->event_callback(
        page->user_data,
        WATER_WPE_EVENT_LOAD_FAILED,
        error->message,
        "",
        0);
    return FALSE;
}

static gboolean water_wpe_tls_failed(
    WebKitWebView *web_view,
    const char *failing_uri,
    GTlsCertificate *certificate,
    GTlsCertificateFlags errors,
    WaterWpePage *page)
{
    (void)web_view;
    (void)certificate;
    char *message = g_strdup_printf("TLS certificate errors: 0x%x", errors);
    page->event_callback(
        page->user_data,
        WATER_WPE_EVENT_TLS_FAILED,
        failing_uri,
        message,
        0);
    g_free(message);
    return FALSE;
}

static void water_wpe_evaluate_without_result(
    WaterWpePage *page,
    const char *script)
{
    webkit_web_view_evaluate_javascript(
        page->web_view,
        script,
        -1,
        NULL,
        NULL,
        NULL,
        NULL,
        NULL);
}

static void water_wpe_script_message(
    WebKitUserContentManager *manager,
    JSCValue *value,
    WaterWpePage *page)
{
    (void)manager;
    /* The envelope is forwarded verbatim: it is parsed once, in Rust, so its
     * format is defined in exactly one place. */
    char *envelope = jsc_value_to_string(value);
    WaterWpeBytes reply = page->message_callback(page->user_data, envelope);
    if (reply.data != NULL && reply.len > 0) {
        char *script = g_strndup((const char *)reply.data, reply.len);
        water_wpe_evaluate_without_result(page, script);
        g_free(script);
    }
    if (reply.destroy)
        reply.destroy(reply.user_data);
    g_free(envelope);
}

WaterWpePage *water_wpe_page_new(
    WaterWpeRuntime *runtime,
    WaterWpeEventCallback event_callback,
    WaterWpeFrameCallback frame_callback,
    WaterWpeMessageCallback message_callback,
    void *user_data,
    WaterWpeDestroyNotify destroy_user_data,
    char **error)
{
    g_assert(runtime != NULL);
    g_assert(event_callback != NULL);
    g_assert(frame_callback != NULL);
    g_assert(message_callback != NULL);
    g_assert(destroy_user_data != NULL);
    g_assert(error != NULL);
    *error = NULL;

    WaterWpePage *page = g_new0(WaterWpePage, 1);
    page->runtime = runtime;
    page->event_callback = event_callback;
    page->frame_callback = frame_callback;
    page->message_callback = message_callback;
    page->user_data = user_data;
    page->destroy_user_data = destroy_user_data;
    page->redirects_enabled = TRUE;
    page->content_manager = webkit_user_content_manager_new();
    gboolean registered = webkit_user_content_manager_register_script_message_handler(
        page->content_manager,
        "__waterui",
        NULL);
    g_assert(registered);
    g_signal_connect(
        page->content_manager,
        "script-message-received::__waterui",
        G_CALLBACK(water_wpe_script_message),
        page);

    page->web_view = WEBKIT_WEB_VIEW(g_object_new(
        WEBKIT_TYPE_WEB_VIEW,
        "display",
        runtime->display,
        "user-content-manager",
        page->content_manager,
        NULL));
    WebKitSettings *settings = webkit_web_view_get_settings(page->web_view);
    webkit_settings_set_hardware_acceleration_policy(
        settings,
        WEBKIT_HARDWARE_ACCELERATION_POLICY_ALWAYS);
    page->view = webkit_web_view_get_wpe_view(page->web_view);
    g_assert(WATER_IS_VIEW(page->view));
    ((WaterView *)page->view)->page = page;
    page->toplevel = wpe_display_create_toplevel(runtime->display, 1);
    wpe_view_set_toplevel(page->view, page->toplevel);
    wpe_toplevel_resize(page->toplevel, 1, 1);
    wpe_view_set_visible(page->view, TRUE);

    g_signal_connect(
        page->web_view,
        "load-changed",
        G_CALLBACK(water_wpe_load_changed),
        page);
    g_signal_connect(
        page->web_view,
        "notify::estimated-load-progress",
        G_CALLBACK(water_wpe_progress_changed),
        page);
    g_signal_connect(
        page->web_view,
        "notify::can-go-back",
        G_CALLBACK(water_wpe_history_changed),
        page);
    g_signal_connect(
        page->web_view,
        "notify::can-go-forward",
        G_CALLBACK(water_wpe_history_changed),
        page);
    g_signal_connect(
        page->web_view,
        "decide-policy",
        G_CALLBACK(water_wpe_decide_policy),
        page);
    g_signal_connect(
        page->web_view,
        "load-failed",
        G_CALLBACK(water_wpe_load_failed),
        page);
    g_signal_connect(
        page->web_view,
        "load-failed-with-tls-errors",
        G_CALLBACK(water_wpe_tls_failed),
        page);
    return page;
}

void water_wpe_page_free(WaterWpePage *page)
{
    g_assert(page != NULL);
    ((WaterView *)page->view)->page = NULL;
    wpe_view_closed(page->view);
    wpe_toplevel_closed(page->toplevel);
    g_object_unref(page->toplevel);
    g_object_unref(page->web_view);
    g_object_unref(page->content_manager);
    page->destroy_user_data(page->user_data);
    g_free(page->last_uri);
    g_free(page);
}

void water_wpe_page_load_uri(WaterWpePage *page, const char *uri)
{
    webkit_web_view_load_uri(page->web_view, uri);
}

void water_wpe_page_go_back(WaterWpePage *page)
{
    webkit_web_view_go_back(page->web_view);
}

void water_wpe_page_go_forward(WaterWpePage *page)
{
    webkit_web_view_go_forward(page->web_view);
}

void water_wpe_page_stop(WaterWpePage *page)
{
    webkit_web_view_stop_loading(page->web_view);
}

void water_wpe_page_reload(WaterWpePage *page)
{
    webkit_web_view_reload(page->web_view);
}

bool water_wpe_page_can_go_back(WaterWpePage *page)
{
    return webkit_web_view_can_go_back(page->web_view);
}

bool water_wpe_page_can_go_forward(WaterWpePage *page)
{
    return webkit_web_view_can_go_forward(page->web_view);
}

void water_wpe_page_set_redirects_enabled(WaterWpePage *page, bool enabled)
{
    page->redirects_enabled = enabled;
}

void water_wpe_page_set_user_agent(WaterWpePage *page, const char *user_agent)
{
    webkit_settings_set_user_agent(
        webkit_web_view_get_settings(page->web_view),
        user_agent);
}

void water_wpe_page_resize(
    WaterWpePage *page,
    uint32_t width,
    uint32_t height,
    double scale)
{
    g_assert_cmpuint(width, >, 0);
    g_assert_cmpuint(height, >, 0);
    g_assert_cmpfloat(scale, >, 0);
    wpe_toplevel_scale_changed(page->toplevel, scale);
    gboolean resized = wpe_toplevel_resize(
        page->toplevel,
        (int)width,
        (int)height);
    g_assert(resized);
}

void water_wpe_page_set_focus(WaterWpePage *page, bool focused)
{
    if (focused)
        wpe_view_focus_in(page->view);
    else
        wpe_view_focus_out(page->view);
}

void water_wpe_page_pointer_button(
    WaterWpePage *page,
    bool pressed,
    uint32_t button,
    double x,
    double y,
    uint32_t modifiers,
    uint32_t time_ms)
{
    WPEEventType type =
        pressed ? WPE_EVENT_POINTER_DOWN : WPE_EVENT_POINTER_UP;
    guint press_count = wpe_view_compute_press_count(
        page->view,
        x,
        y,
        button,
        time_ms);
    WPEEvent *event = wpe_event_pointer_button_new(
        type,
        page->view,
        WPE_INPUT_SOURCE_MOUSE,
        time_ms,
        (WPEModifiers)modifiers,
        button,
        x,
        y,
        press_count);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_page_pointer_move(
    WaterWpePage *page,
    double x,
    double y,
    double delta_x,
    double delta_y,
    uint32_t modifiers,
    uint32_t time_ms)
{
    WPEEvent *event = wpe_event_pointer_move_new(
        WPE_EVENT_POINTER_MOVE,
        page->view,
        WPE_INPUT_SOURCE_MOUSE,
        time_ms,
        (WPEModifiers)modifiers,
        x,
        y,
        delta_x,
        delta_y);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_page_scroll(
    WaterWpePage *page,
    double x,
    double y,
    double delta_x,
    double delta_y,
    bool precise,
    bool stopped,
    uint32_t modifiers,
    uint32_t time_ms)
{
    WPEEvent *event = wpe_event_scroll_new(
        page->view,
        WPE_INPUT_SOURCE_TOUCHPAD,
        time_ms,
        (WPEModifiers)modifiers,
        delta_x,
        delta_y,
        precise,
        stopped,
        x,
        y);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_page_key(
    WaterWpePage *page,
    bool pressed,
    uint32_t keycode,
    uint32_t keyval,
    uint32_t modifiers,
    uint32_t time_ms)
{
    WPEEvent *event = wpe_event_keyboard_new(
        pressed ? WPE_EVENT_KEYBOARD_KEY_DOWN : WPE_EVENT_KEYBOARD_KEY_UP,
        page->view,
        WPE_INPUT_SOURCE_KEYBOARD,
        time_ms,
        (WPEModifiers)modifiers,
        keycode,
        keyval);
    wpe_view_event(page->view, event);
    wpe_event_unref(event);
}

void water_wpe_page_evaluate(WaterWpePage *page, const char *script)
{
    g_assert(page != NULL);
    g_assert(script != NULL);
    water_wpe_evaluate_without_result(page, script);
}

void water_wpe_page_add_script(
    WaterWpePage *page,
    const char *script,
    uint32_t injection_time)
{
    g_assert_cmpuint(injection_time, <=, 1);
    WebKitUserScript *user_script = webkit_user_script_new(
        script,
        WEBKIT_USER_CONTENT_INJECT_ALL_FRAMES,
        injection_time == 0
            ? WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START
            : WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_END,
        NULL,
        NULL);
    webkit_user_content_manager_add_script(page->content_manager, user_script);
    webkit_user_script_unref(user_script);
}

typedef struct {
    WaterWpeResultCallback callback;
    void *user_data;
} WaterWpeAsyncResult;

static void water_wpe_cookie_added(
    GObject *object,
    GAsyncResult *result,
    gpointer user_data)
{
    (void)user_data;
    GError *error = NULL;
    gboolean success = webkit_cookie_manager_add_cookie_finish(
        WEBKIT_COOKIE_MANAGER(object),
        result,
        &error);
    if (!success)
        g_error("WPE cookie insertion failed: %s", error->message);
    g_clear_error(&error);
}

void water_wpe_page_set_cookie(WaterWpePage *page, const char *cookie)
{
    const char *uri = webkit_web_view_get_uri(page->web_view);
    g_assert(uri != NULL);
    SoupCookie *parsed = soup_cookie_parse(cookie, uri);
    g_assert(parsed != NULL);
    WebKitCookieManager *manager = webkit_website_data_manager_get_cookie_manager(
        webkit_web_view_get_website_data_manager(page->web_view));
    webkit_cookie_manager_add_cookie(
        manager,
        parsed,
        NULL,
        water_wpe_cookie_added,
        NULL);
    soup_cookie_free(parsed);
}

static const char *water_wpe_same_site(SoupSameSitePolicy policy)
{
    switch (policy) {
    case SOUP_SAME_SITE_POLICY_NONE:
        return "None";
    case SOUP_SAME_SITE_POLICY_LAX:
        return "Lax";
    case SOUP_SAME_SITE_POLICY_STRICT:
        return "Strict";
    }
    g_assert_not_reached();
}

static void water_wpe_cookies_ready(
    GObject *object,
    GAsyncResult *result,
    gpointer user_data)
{
    WaterWpeAsyncResult *async = user_data;
    GError *error = NULL;
    GList *cookies = webkit_cookie_manager_get_cookies_finish(
        WEBKIT_COOKIE_MANAGER(object),
        result,
        &error);
    if (error) {
        async->callback(
            async->user_data,
            false,
            error->message,
            strlen(error->message));
        g_error_free(error);
        g_free(async);
        return;
    }
    JsonBuilder *builder = json_builder_new();
    json_builder_begin_array(builder);
    for (GList *item = cookies; item; item = item->next) {
        SoupCookie *cookie = item->data;
        json_builder_begin_object(builder);
#define WATER_COOKIE_STRING(member, value) \
        json_builder_set_member_name(builder, member); \
        json_builder_add_string_value(builder, value)
#define WATER_COOKIE_BOOL(member, value) \
        json_builder_set_member_name(builder, member); \
        json_builder_add_boolean_value(builder, value)
        WATER_COOKIE_STRING("name", soup_cookie_get_name(cookie));
        WATER_COOKIE_STRING("value", soup_cookie_get_value(cookie));
        WATER_COOKIE_STRING("domain", soup_cookie_get_domain(cookie));
        WATER_COOKIE_STRING("path", soup_cookie_get_path(cookie));
        WATER_COOKIE_BOOL("secure", soup_cookie_get_secure(cookie));
        WATER_COOKIE_BOOL("http_only", soup_cookie_get_http_only(cookie));
        json_builder_set_member_name(builder, "same_site");
        json_builder_add_string_value(
            builder,
            water_wpe_same_site(soup_cookie_get_same_site_policy(cookie)));
        json_builder_set_member_name(builder, "expires");
        GDateTime *expires = soup_cookie_get_expires(cookie);
        if (expires)
            json_builder_add_int_value(builder, g_date_time_to_unix(expires));
        else
            json_builder_add_null_value(builder);
        json_builder_end_object(builder);
#undef WATER_COOKIE_BOOL
#undef WATER_COOKIE_STRING
    }
    json_builder_end_array(builder);
    JsonGenerator *generator = json_generator_new();
    JsonNode *root = json_builder_get_root(builder);
    json_generator_set_root(generator, root);
    gsize length = 0;
    char *json = json_generator_to_data(generator, &length);
    async->callback(async->user_data, true, json, length);
    g_free(json);
    json_node_free(root);
    g_object_unref(generator);
    g_object_unref(builder);
    g_list_free_full(cookies, (GDestroyNotify)soup_cookie_free);
    g_free(async);
}

void water_wpe_page_get_cookies(
    WaterWpePage *page,
    WaterWpeResultCallback callback,
    void *user_data)
{
    const char *uri = webkit_web_view_get_uri(page->web_view);
    g_assert(uri != NULL);
    WebKitCookieManager *manager = webkit_website_data_manager_get_cookie_manager(
        webkit_web_view_get_website_data_manager(page->web_view));
    WaterWpeAsyncResult *async = g_new0(WaterWpeAsyncResult, 1);
    async->callback = callback;
    async->user_data = user_data;
    webkit_cookie_manager_get_cookies(
        manager,
        uri,
        NULL,
        water_wpe_cookies_ready,
        async);
}

static void water_wpe_javascript_ready(
    GObject *object,
    GAsyncResult *result,
    gpointer user_data)
{
    WaterWpeAsyncResult *async = user_data;
    GError *error = NULL;
    JSCValue *value = webkit_web_view_evaluate_javascript_finish(
        WEBKIT_WEB_VIEW(object),
        result,
        &error);
    if (!value) {
        async->callback(
            async->user_data,
            false,
            error->message,
            strlen(error->message));
        g_error_free(error);
        g_free(async);
        return;
    }
    char *json = jsc_value_to_json(value, 0);
    if (!json)
        json = jsc_value_to_string(value);
    async->callback(async->user_data, true, json, strlen(json));
    g_free(json);
    g_object_unref(value);
    g_free(async);
}

void water_wpe_page_run_javascript(
    WaterWpePage *page,
    const char *script,
    WaterWpeResultCallback callback,
    void *user_data)
{
    WaterWpeAsyncResult *async = g_new0(WaterWpeAsyncResult, 1);
    async->callback = callback;
    async->user_data = user_data;
    webkit_web_view_evaluate_javascript(
        page->web_view,
        script,
        -1,
        NULL,
        NULL,
        NULL,
        water_wpe_javascript_ready,
        async);
}

void water_wpe_frame_presented(void *user_data)
{
    WaterWpeFrameToken *token = user_data;
    g_main_context_invoke_full(
        token->context,
        G_PRIORITY_HIGH,
        water_wpe_frame_presented_on_main,
        water_wpe_frame_token_ref(token),
        NULL);
}

void water_wpe_frame_release(void *user_data, int release_fence_fd)
{
    WaterWpeFrameToken *token = user_data;
    WaterWpeRelease *release = g_new0(WaterWpeRelease, 1);
    release->token = token;
    release->release_fence_fd = release_fence_fd;
    g_main_context_invoke_full(
        token->context,
        G_PRIORITY_HIGH,
        water_wpe_frame_released_on_main,
        release,
        NULL);
    water_wpe_frame_token_unref(token);
}
