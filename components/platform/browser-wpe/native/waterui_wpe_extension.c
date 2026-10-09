#include <gmodule.h>
#include <jsc/jsc.h>
#include <string.h>
#include <unistd.h>
#include <wpe/webkit-web-process-extension.h>

typedef struct {
    char *initial_origin_wire;
    WebKitScriptWorld *bridge_world;
} WaterWpeProcess;

typedef struct {
    gatomicrefcount references;
    char *origin_wire;
} WaterWpeExtensionPage;

typedef struct {
    WaterWpeExtensionPage *page;
    JSCValue *isolated_send;
    char *origin;
} WaterWpeBinding;

typedef struct {
    gatomicrefcount references;
    JSCContext *context;
    JSCValue *resolve;
    JSCValue *reject;
    gboolean settled;
} WaterWpePromise;

static void water_wpe_process_free(gpointer user_data)
{
    WaterWpeProcess *process = user_data;
    g_object_unref(process->bridge_world);
    g_free(process->initial_origin_wire);
    g_free(process);
}

static WaterWpeExtensionPage *water_wpe_extension_page_ref(
    WaterWpeExtensionPage *page)
{
    g_atomic_ref_count_inc(&page->references);
    return page;
}

static void water_wpe_extension_page_unref(WaterWpeExtensionPage *page)
{
    if (!g_atomic_ref_count_dec(&page->references))
        return;
    g_free(page->origin_wire);
    g_free(page);
}

static void water_wpe_extension_page_destroy(gpointer user_data)
{
    water_wpe_extension_page_unref(user_data);
}

static gboolean water_wpe_origin_allowed(
    const char *wire,
    const char *origin)
{
    const char *rule = wire;
    while (rule && *rule) {
        const char *end = strchr(rule, '\n');
        gsize length = end ? (gsize)(end - rule) : strlen(rule);
        if (length == 1 && rule[0] == '*')
            return TRUE;
        if (length == 5 && strncmp(rule, "file:", length) == 0 &&
            g_str_has_prefix(origin, "file://"))
            return TRUE;
        if (strlen(origin) == length && strncmp(rule, origin, length) == 0)
            return TRUE;
        rule = end ? end + 1 : NULL;
    }
    return FALSE;
}

static void water_wpe_binding_free(gpointer user_data)
{
    WaterWpeBinding *binding = user_data;
    g_object_unref(binding->isolated_send);
    water_wpe_extension_page_unref(binding->page);
    g_free(binding->origin);
    g_free(binding);
}

static WaterWpePromise *water_wpe_promise_ref(WaterWpePromise *promise)
{
    g_atomic_ref_count_inc(&promise->references);
    return promise;
}

static void water_wpe_promise_unref(gpointer user_data)
{
    WaterWpePromise *promise = user_data;
    if (!g_atomic_ref_count_dec(&promise->references))
        return;
    g_clear_object(&promise->resolve);
    g_clear_object(&promise->reject);
    g_clear_object(&promise->context);
    g_free(promise);
}

static void water_wpe_promise_executor(
    JSCValue *resolve,
    JSCValue *reject,
    gpointer user_data)
{
    WaterWpePromise *promise = user_data;
    promise->resolve = g_object_ref(resolve);
    promise->reject = g_object_ref(reject);
}

static void water_wpe_promise_clear(WaterWpePromise *promise)
{
    promise->settled = TRUE;
    g_clear_object(&promise->resolve);
    g_clear_object(&promise->reject);
    g_clear_object(&promise->context);
}

static char *water_wpe_exception_message(JSCException *exception)
{
    const char *message = jsc_exception_get_message(exception);
    return g_strdup(message ? message : "WaterUI bridge dispatch failed");
}

static JSCValue *water_wpe_settle_default_promise(
    JSCValue *value,
    WaterWpePromise *promise,
    gboolean success)
{
    JSCContext *isolated_context = jsc_value_get_context(value);
    if (promise->settled || !promise->context)
        return jsc_value_new_undefined(isolated_context);
    char *text = success ? jsc_value_to_json(value, 0) : jsc_value_to_string(value);
    if (!text)
        text = g_strdup(success ? "null" : "WaterUI bridge request failed");
    JSCValue *primitive = jsc_value_new_string(promise->context, text);
    JSCValue *arguments[] = { primitive };
    JSCValue *settled = jsc_value_function_callv(
        success ? promise->resolve : promise->reject,
        G_N_ELEMENTS(arguments),
        arguments);
    if (settled)
        g_object_unref(settled);
    g_object_unref(primitive);
    g_free(text);
    water_wpe_promise_clear(promise);
    return jsc_value_new_undefined(isolated_context);
}

static JSCValue *water_wpe_promise_fulfilled(
    JSCValue *value,
    WaterWpePromise *promise)
{
    return water_wpe_settle_default_promise(value, promise, TRUE);
}

static JSCValue *water_wpe_promise_rejected(
    JSCValue *value,
    WaterWpePromise *promise)
{
    return water_wpe_settle_default_promise(value, promise, FALSE);
}

static void water_wpe_reject_default_promise(
    WaterWpePromise *promise,
    const char *message)
{
    if (promise->settled || !promise->context)
        return;
    JSCValue *reason = jsc_value_new_string(promise->context, message);
    JSCValue *arguments[] = { reason };
    JSCValue *settled =
        jsc_value_function_callv(promise->reject, G_N_ELEMENTS(arguments), arguments);
    if (settled)
        g_object_unref(settled);
    g_object_unref(reason);
    water_wpe_promise_clear(promise);
}

static JSCValue *water_wpe_native_send(
    JSCValue *envelope_value,
    WaterWpeBinding *binding)
{
    JSCContext *context = jsc_value_get_context(envelope_value);
    WaterWpePromise *promise = g_new0(WaterWpePromise, 1);
    g_atomic_ref_count_init(&promise->references);
    promise->context = g_object_ref(context);
    JSCValue *default_promise = jsc_value_new_promise(
        context,
        water_wpe_promise_executor,
        promise);
    if (!jsc_value_is_string(envelope_value) ||
        !water_wpe_origin_allowed(binding->page->origin_wire, binding->origin)) {
        water_wpe_reject_default_promise(
            promise, "this document is not allowed to use the WaterUI bridge");
        water_wpe_promise_unref(promise);
        return default_promise;
    }

    char *envelope = jsc_value_to_string(envelope_value);
    JSCContext *isolated_context =
        jsc_value_get_context(binding->isolated_send);
    JSCValue *origin_value = jsc_value_new_string(isolated_context, binding->origin);
    JSCValue *isolated_envelope =
        jsc_value_new_string(isolated_context, envelope);
    JSCValue *arguments[] = { origin_value, isolated_envelope };
    JSCValue *isolated_promise =
        jsc_value_function_callv(binding->isolated_send, G_N_ELEMENTS(arguments), arguments);
    JSCException *exception = jsc_context_get_exception(isolated_context);
    if (exception) {
        char *message = water_wpe_exception_message(exception);
        water_wpe_reject_default_promise(
            promise, message);
        g_free(message);
        jsc_context_clear_exception(isolated_context);
        if (isolated_promise)
            g_object_unref(isolated_promise);
        g_object_unref(origin_value);
        g_object_unref(isolated_envelope);
        g_free(envelope);
        water_wpe_promise_unref(promise);
        return default_promise;
    }
    if (!isolated_promise) {
        water_wpe_reject_default_promise(
            promise, "WaterUI bridge dispatch did not return a promise");
        g_object_unref(origin_value);
        g_object_unref(isolated_envelope);
        g_free(envelope);
        water_wpe_promise_unref(promise);
        return default_promise;
    }
    JSCValue *fulfilled = jsc_value_new_function(
        isolated_context,
        NULL,
        G_CALLBACK(water_wpe_promise_fulfilled),
        water_wpe_promise_ref(promise),
        water_wpe_promise_unref,
        JSC_TYPE_VALUE,
        1,
        JSC_TYPE_VALUE);
    JSCValue *rejected = jsc_value_new_function(
        isolated_context,
        NULL,
        G_CALLBACK(water_wpe_promise_rejected),
        water_wpe_promise_ref(promise),
        water_wpe_promise_unref,
        JSC_TYPE_VALUE,
        1,
        JSC_TYPE_VALUE);
    JSCValue *continuations[] = { fulfilled, rejected };
    JSCValue *chained = jsc_value_object_invoke_methodv(
        isolated_promise,
        "then",
        G_N_ELEMENTS(continuations),
        continuations);
    exception = jsc_context_get_exception(isolated_context);
    if (exception) {
        char *message = water_wpe_exception_message(exception);
        water_wpe_reject_default_promise(promise, message);
        g_free(message);
        jsc_context_clear_exception(isolated_context);
    } else if (!chained) {
        water_wpe_reject_default_promise(
            promise,
            "WaterUI bridge dispatch did not return a promise");
    }
    if (chained)
        g_object_unref(chained);
    g_object_unref(rejected);
    g_object_unref(fulfilled);
    g_object_unref(isolated_promise);
    g_object_unref(isolated_envelope);
    g_object_unref(origin_value);
    g_free(envelope);
    water_wpe_promise_unref(promise);
    return default_promise;
}

static char *water_wpe_frame_origin(JSCContext *isolated_context)
{
    JSCValue *origin = jsc_context_evaluate(
        isolated_context,
        "globalThis.origin",
        -1);
    JSCException *exception = jsc_context_get_exception(isolated_context);
    if (exception) {
        jsc_context_clear_exception(isolated_context);
        if (origin)
            g_object_unref(origin);
        return g_strdup("");
    }
    if (!origin || !jsc_value_is_string(origin)) {
        if (origin)
            g_object_unref(origin);
        return g_strdup("");
    }
    char *serialized = jsc_value_to_string(origin);
    g_object_unref(origin);
    if (g_str_equal(serialized, "null")) {
        g_free(serialized);
        return g_strdup("");
    }
    return serialized;
}

static void water_wpe_window_object_cleared(
    WebKitScriptWorld *world,
    WebKitWebPage *web_page,
    WebKitFrame *frame,
    gpointer user_data)
{
    (void)world;
    WaterWpeProcess *process = user_data;
    if (!webkit_frame_is_main_frame(frame))
        return;

    WaterWpeExtensionPage *page =
        g_object_get_data(G_OBJECT(web_page), "waterui.bridge.page");
    if (!page)
        return;

    JSCContext *isolated_context =
        webkit_frame_get_js_context_for_script_world(frame, process->bridge_world);
    JSCContext *default_context = webkit_frame_get_js_context_for_script_world(
        frame,
        webkit_script_world_get_default());
    if (!isolated_context || !default_context) {
        if (isolated_context)
            g_object_unref(isolated_context);
        if (default_context)
            g_object_unref(default_context);
        return;
    }
    char *origin = water_wpe_frame_origin(isolated_context);
    JSCValue *isolated_send = jsc_context_evaluate(
        isolated_context,
        "(function(origin,envelope){return globalThis.webkit.messageHandlers."
        "__waterui.postMessage({origin:origin,envelope:envelope});})",
        -1);
    JSCException *exception = jsc_context_get_exception(isolated_context);
    if (exception || !isolated_send || !jsc_value_is_function(isolated_send)) {
        if (exception)
            jsc_context_clear_exception(isolated_context);
        if (isolated_send)
            g_object_unref(isolated_send);
        g_free(origin);
        g_object_unref(default_context);
        g_object_unref(isolated_context);
        return;
    }

    WaterWpeBinding *binding = g_new0(WaterWpeBinding, 1);
    binding->page = water_wpe_extension_page_ref(page);
    binding->isolated_send = isolated_send;
    binding->origin = origin;
    JSCValue *global = jsc_context_get_global_object(default_context);
    JSCValue *native_send = jsc_value_new_function(
        default_context,
        "__wateruiNativeSend",
        G_CALLBACK(water_wpe_native_send),
        binding,
        water_wpe_binding_free,
        JSC_TYPE_VALUE,
        1,
        JSC_TYPE_VALUE);
    jsc_value_object_define_property_data(
        global,
        "__wateruiNativeSend",
        JSC_VALUE_PROPERTY_CONFIGURABLE,
        native_send);
    g_object_unref(native_send);
    g_object_unref(global);
    g_object_unref(default_context);
    g_object_unref(isolated_context);
}

static gboolean water_wpe_user_message_received(
    WebKitWebPage *web_page,
    WebKitUserMessage *message,
    gpointer user_data)
{
    (void)user_data;
    const char *name = webkit_user_message_get_name(message);
    if (g_str_equal(name, "waterui.bridge-origins")) {
        WaterWpeExtensionPage *page =
            g_object_get_data(G_OBJECT(web_page), "waterui.bridge.page");
        GVariant *parameters = webkit_user_message_get_parameters(message);
        if (page && parameters &&
            g_variant_is_of_type(parameters, G_VARIANT_TYPE_STRING)) {
            g_free(page->origin_wire);
            page->origin_wire = g_variant_dup_string(parameters, NULL);
        }
        webkit_user_message_send_reply(
            message,
            webkit_user_message_new("waterui.bridge-origins-ack", NULL));
        return TRUE;
    }
    if (g_str_equal(name, "waterui.bridge-process-id")) {
        webkit_user_message_send_reply(
            message,
            webkit_user_message_new(
                "waterui.bridge-process-id",
                g_variant_new_uint64((guint64)getpid())));
        return TRUE;
    }
    return FALSE;
}

static void water_wpe_page_created(
    WebKitWebProcessExtension *extension,
    WebKitWebPage *web_page,
    gpointer user_data)
{
    (void)extension;
    WaterWpeProcess *process = user_data;
    WaterWpeExtensionPage *page = g_new0(WaterWpeExtensionPage, 1);
    g_atomic_ref_count_init(&page->references);
    page->origin_wire = g_strdup(process->initial_origin_wire);
    g_object_set_data_full(
        G_OBJECT(web_page),
        "waterui.bridge.page",
        page,
        water_wpe_extension_page_destroy);
    g_signal_connect(
        web_page,
        "user-message-received",
        G_CALLBACK(water_wpe_user_message_received),
        NULL);
}

G_MODULE_EXPORT void webkit_web_process_extension_initialize_with_user_data(
    WebKitWebProcessExtension *extension,
    const GVariant *user_data)
{
    WaterWpeProcess *process = g_new0(WaterWpeProcess, 1);
    const char *initial_wire = "";
    if (user_data && g_variant_is_of_type(user_data, G_VARIANT_TYPE_STRING))
        initial_wire = g_variant_get_string(user_data, NULL);
    process->initial_origin_wire = g_strdup(initial_wire);
    process->bridge_world = webkit_script_world_new_with_name("waterui.bridge");
    g_object_set_data_full(
        G_OBJECT(extension),
        "waterui.bridge.process",
        process,
        water_wpe_process_free);
    g_signal_connect(
        extension,
        "page-created",
        G_CALLBACK(water_wpe_page_created),
        process);
    g_signal_connect(
        webkit_script_world_get_default(),
        "window-object-cleared",
        G_CALLBACK(water_wpe_window_object_cleared),
        process);
}
