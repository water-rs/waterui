#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum WaterUIStackMode {
  WaterUIStackMode_Auto,
  WaterUIStackMode_Vertical,
  WaterUIStackMode_Horizonal,
  WaterUIStackMode_Layered,
} WaterUIStackMode;

typedef struct WaterUIData {
  uint8_t *head;
  uintptr_t len;
} WaterUIData;

typedef struct WaterUIComputedData {
  uintptr_t inner[2];
} WaterUIComputedData;

typedef struct WaterUIClosure {
  void *data;
  void (*call)(const void*);
  void (*free)(void*);
} WaterUIClosure;

typedef struct WaterUIUtf8Data {
  uint8_t *head;
  uintptr_t len;
} WaterUIUtf8Data;

typedef struct WaterUIComputedStr {
  uintptr_t inner[2];
} WaterUIComputedStr;

typedef intptr_t WaterUIInt;

typedef struct WaterUIComputedInt {
  uintptr_t inner[2];
} WaterUIComputedInt;

typedef struct WaterUIComputedBool {
  uintptr_t inner[2];
} WaterUIComputedBool;

typedef struct WaterUIAnyView {
  uintptr_t inner[2];
} WaterUIAnyView;

typedef struct WaterUIComputedView {
  uintptr_t inner[2];
} WaterUIComputedView;

typedef struct WaterUITypeId {
  uint64_t inner[2];
} WaterUITypeId;

typedef struct WaterUIBindingStr {
  uintptr_t inner[1];
} WaterUIBindingStr;

typedef struct WaterUIBindingInt {
  uintptr_t inner[1];
} WaterUIBindingInt;

typedef struct WaterUIBridge {
  uintptr_t inner[1];
} WaterUIBridge;

typedef struct WaterUIEnvironment {
  uintptr_t inner[1];
} WaterUIEnvironment;

typedef struct WaterUIError {
  struct WaterUIUtf8Data msg;
} WaterUIError;

typedef struct WaterUIAction {
  uintptr_t inner[2];
} WaterUIAction;

typedef struct WaterUIButton {
  struct WaterUIAnyView label;
  struct WaterUIAction action;
} WaterUIButton;

typedef struct WaterUITextField {
  struct WaterUIAnyView label;
  struct WaterUIBindingStr value;
  struct WaterUIComputedStr prompt;
} WaterUITextField;

typedef struct WaterUIErrorViewBuilder {
  uintptr_t inner[2];
} WaterUIErrorViewBuilder;

typedef struct WaterUIRemoteImage {
  struct WaterUIComputedStr url;
  struct WaterUIAnyView loading;
  struct WaterUIErrorViewBuilder error;
} WaterUIRemoteImage;

typedef struct WaterUIViews {
  struct WaterUIAnyView *head;
  uintptr_t len;
} WaterUIViews;

typedef struct WaterUIStack {
  struct WaterUIViews views;
  enum WaterUIStackMode mode;
} WaterUIStack;

typedef struct WaterUIText {
  struct WaterUIComputedStr content;
  struct WaterUIComputedBool selection;
} WaterUIText;

typedef struct WaterUIEdge {
  double top;
  double right;
  double bottom;
  double left;
} WaterUIEdge;

typedef struct WaterUIPadding {
  struct WaterUIEdge _inner;
} WaterUIPadding;

typedef struct WaterUIWithValue_Padding {
  struct WaterUIPadding value;
  struct WaterUIAnyView content;
} WaterUIWithValue_Padding;

typedef struct WaterUIApp {
  struct WaterUIAnyView content;
  struct WaterUIEnvironment env;
} WaterUIApp;

typedef struct WaterUIAppClosure {
  void *data;
  void (*call)(const void*, struct WaterUIApp);
  void (*free)(void*);
} WaterUIAppClosure;

struct WaterUIData waterui_read_computed_data(const struct WaterUIComputedData *computed);

uintptr_t waterui_subscribe_computed_data(const struct WaterUIComputedData *computed,
                                          struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_data(const struct WaterUIComputedData *computed, uintptr_t id);

void waterui_drop_computed_data(struct WaterUIComputedData computed);

struct WaterUIUtf8Data waterui_read_computed_str(const struct WaterUIComputedStr *computed);

uintptr_t waterui_subscribe_computed_str(const struct WaterUIComputedStr *computed,
                                         struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_str(const struct WaterUIComputedStr *computed, uintptr_t id);

void waterui_drop_computed_str(struct WaterUIComputedStr computed);

WaterUIInt waterui_read_computed_int(const struct WaterUIComputedInt *computed);

uintptr_t waterui_subscribe_computed_int(const struct WaterUIComputedInt *computed,
                                         struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_int(const struct WaterUIComputedInt *computed, uintptr_t id);

void waterui_drop_computed_int(struct WaterUIComputedInt computed);

bool waterui_read_computed_bool(const struct WaterUIComputedBool *computed);

uintptr_t waterui_subscribe_computed_bool(const struct WaterUIComputedBool *computed,
                                          struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_bool(const struct WaterUIComputedBool *computed, uintptr_t id);

void waterui_drop_computed_bool(struct WaterUIComputedBool computed);

struct WaterUIAnyView waterui_read_computed_view(const struct WaterUIComputedView *computed);

uintptr_t waterui_subscribe_computed_view(const struct WaterUIComputedView *computed,
                                          struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_view(const struct WaterUIComputedView *computed, uintptr_t id);

void waterui_drop_computed_view(struct WaterUIComputedView computed);

struct WaterUIComputedView waterui_view_force_as_dynamic_view(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_dynamic_view_id(void);

struct WaterUIUtf8Data waterui_read_binding_str(const struct WaterUIBindingStr *binding);

void waterui_write_binding_str(const struct WaterUIBindingStr *binding,
                               struct WaterUIUtf8Data value);

uintptr_t waterui_subscribe_binding_str(const struct WaterUIBindingStr *binding,
                                        struct WaterUIClosure subscriber);

void waterui_unsubscribe_binding_str(const struct WaterUIBindingStr *binding, uintptr_t id);

void waterui_drop_binding_str(struct WaterUIBindingStr binding);

WaterUIInt waterui_read_binding_int(const struct WaterUIBindingInt *binding);

void waterui_write_binding_int(const struct WaterUIBindingInt *binding, WaterUIInt value);

uintptr_t waterui_subscribe_binding_int(const struct WaterUIBindingInt *binding,
                                        struct WaterUIClosure subscriber);

void waterui_unsubscribe_binding_int(const struct WaterUIBindingInt *binding, uintptr_t id);

void waterui_drop_binding_int(struct WaterUIBindingInt binding);

int8_t waterui_send_to_bridge(const struct WaterUIBridge *bridge, struct WaterUIClosure f);

struct WaterUIBridge waterui_create_bridge(struct WaterUIEnvironment *env);

struct WaterUIBridge waterui_clone_bridge(const struct WaterUIBridge *bridge);

void waterui_drop_bridge(struct WaterUIBridge bridge);

struct WaterUIError waterui_error(struct WaterUIUtf8Data msg);

struct WaterUITypeId waterui_view_id(const struct WaterUIAnyView *view);

struct WaterUIAnyView waterui_call_view(struct WaterUIAnyView view, struct WaterUIEnvironment env);

struct WaterUITypeId waterui_view_empty_id(void);

struct WaterUIAnyView waterui_view_force_as_anyview(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_anyview_id(void);

struct WaterUIButton waterui_view_force_as_button(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_button_id(void);

struct WaterUITextField waterui_view_force_as_field(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_field_id(void);

struct WaterUIRemoteImage waterui_view_force_as_remoteimg(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_remoteimg_id(void);

struct WaterUIAnyView waterui_build_error_view(struct WaterUIError error,
                                               struct WaterUIErrorViewBuilder builder);

struct WaterUIStack waterui_view_force_as_stack(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_stack_id(void);

struct WaterUIText waterui_view_force_as_text(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_text_id(void);

struct WaterUIEnvironment waterui_clone_env(const struct WaterUIEnvironment *env);

void waterui_drop_env(struct WaterUIEnvironment env);

void waterui_free_action(struct WaterUIAction action);

void waterui_call_action(const struct WaterUIAction *action,
                         const struct WaterUIEnvironment *environment);

struct WaterUIWithValue_Padding waterui_modifier_force_as_padding(struct WaterUIAnyView view);

struct WaterUITypeId waterui_modifier_padding_id(void);
