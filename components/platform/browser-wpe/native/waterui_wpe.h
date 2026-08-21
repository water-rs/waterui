#ifndef WATERUI_WPE_H
#define WATERUI_WPE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define WATER_WPE_ABI_VERSION 2
#define WATER_WPE_MAX_PLANES 4

typedef struct WaterWpeRuntime WaterWpeRuntime;
typedef struct WaterWpePage WaterWpePage;

typedef struct {
    const uint8_t *data;
    size_t len;
    void *user_data;
    void (*destroy)(void *user_data);
} WaterWpeBytes;

typedef struct {
    void *token;
    uint32_t width;
    uint32_t height;
    uint32_t format;
    uint64_t modifier;
    uint32_t n_planes;
    int fds[WATER_WPE_MAX_PLANES];
    uint32_t offsets[WATER_WPE_MAX_PLANES];
    uint32_t strides[WATER_WPE_MAX_PLANES];
    int rendering_fence_fd;
} WaterWpeFrame;

typedef void (*WaterWpeDestroyNotify)(void *user_data);
typedef void (*WaterWpeEventCallback)(
    void *user_data,
    uint32_t kind,
    const char *first,
    const char *second,
    double number);
typedef void (*WaterWpeFrameCallback)(void *user_data, const WaterWpeFrame *frame);
/* Receives one `waterui.invoke(...)` envelope exactly as the page sent it, and
 * returns the JavaScript that completes the call. The envelope format belongs to
 * `waterui_webview::bridge`; this layer only transports it.
 *
 * `origin` is the calling document's `scheme://host[:port]`, or the empty string
 * when it has none to report — an opaque origin, or a page that has not
 * committed a document yet. It is authenticated by the engine rather than taken
 * from the envelope, which page script writes. */
typedef WaterWpeBytes (*WaterWpeMessageCallback)(
    void *user_data,
    const char *origin,
    const char *envelope);
typedef void (*WaterWpeResultCallback)(
    void *user_data,
    bool success,
    const char *data,
    size_t len);

uint32_t water_wpe_abi_version(void);
WaterWpeRuntime *water_wpe_runtime_new(char **error);
void water_wpe_runtime_free(WaterWpeRuntime *runtime);
bool water_wpe_runtime_iteration(WaterWpeRuntime *runtime);
void water_wpe_string_free(char *string);

WaterWpePage *water_wpe_page_new(
    WaterWpeRuntime *runtime,
    WaterWpeEventCallback event_callback,
    WaterWpeFrameCallback frame_callback,
    WaterWpeMessageCallback message_callback,
    void *user_data,
    WaterWpeDestroyNotify destroy_user_data,
    char **error);
void water_wpe_page_free(WaterWpePage *page);
void water_wpe_page_load_uri(WaterWpePage *page, const char *uri);
void water_wpe_page_go_back(WaterWpePage *page);
void water_wpe_page_go_forward(WaterWpePage *page);
void water_wpe_page_stop(WaterWpePage *page);
void water_wpe_page_reload(WaterWpePage *page);
bool water_wpe_page_can_go_back(WaterWpePage *page);
bool water_wpe_page_can_go_forward(WaterWpePage *page);
void water_wpe_page_set_redirects_enabled(WaterWpePage *page, bool enabled);
void water_wpe_page_set_user_agent(WaterWpePage *page, const char *user_agent);
void water_wpe_page_resize(
    WaterWpePage *page,
    uint32_t width,
    uint32_t height,
    double scale);
void water_wpe_page_set_focus(WaterWpePage *page, bool focused);
void water_wpe_page_pointer_button(
    WaterWpePage *page,
    bool pressed,
    uint32_t button,
    double x,
    double y,
    uint32_t modifiers,
    uint32_t time_ms);
void water_wpe_page_pointer_move(
    WaterWpePage *page,
    double x,
    double y,
    double delta_x,
    double delta_y,
    uint32_t modifiers,
    uint32_t time_ms);
void water_wpe_page_scroll(
    WaterWpePage *page,
    double x,
    double y,
    double delta_x,
    double delta_y,
    bool precise,
    bool stopped,
    uint32_t modifiers,
    uint32_t time_ms);
void water_wpe_page_key(
    WaterWpePage *page,
    bool pressed,
    uint32_t keycode,
    uint32_t keyval,
    uint32_t modifiers,
    uint32_t time_ms);
/* Evaluates `script` and discards its result. Used to settle a page promise
 * after an asynchronous handler has finished. */
void water_wpe_page_evaluate(WaterWpePage *page, const char *script);
/* Installs a document script under `key`, replacing whatever script that key
 * already names. The scripts are injected into the top frame only: the bridge is
 * a capability, and embedding a document does not grant it one. */
void water_wpe_page_add_script(
    WaterWpePage *page,
    const char *key,
    const char *script,
    uint32_t injection_time);
void water_wpe_page_set_cookie(WaterWpePage *page, const char *cookie);
void water_wpe_page_get_cookies(
    WaterWpePage *page,
    WaterWpeResultCallback callback,
    void *user_data);
/* Evaluates `script` as a program and reports its result. A promise comes back
 * as a promise: this is the raw path, so nothing is awaited. */
void water_wpe_page_run_javascript(
    WaterWpePage *page,
    const char *script,
    WaterWpeResultCallback callback,
    void *user_data);
/* Runs `body` as the body of an async function and reports the value its promise
 * resolves with. Every typed evaluation takes this path, because the shared
 * `__wateruiEval` wrapper is async and `webkit_web_view_evaluate_javascript`
 * would hand back the unresolved promise instead of the envelope. */
void water_wpe_page_call_async_javascript(
    WaterWpePage *page,
    const char *body,
    WaterWpeResultCallback callback,
    void *user_data);

void water_wpe_frame_presented(void *token);
void water_wpe_frame_release(void *token, int release_fence_fd);

#ifdef __cplusplus
}
#endif

#endif
