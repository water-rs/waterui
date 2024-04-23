#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum WaterUIProgressStyle {
  WaterUIProgressStyle_Default,
  WaterUIProgressStyle_Circular,
  WaterUIProgressStyle_Linear,
} WaterUIProgressStyle;

typedef enum WaterUIStackMode {
  WaterUIStackMode_Auto,
  WaterUIStackMode_Vertical,
  WaterUIStackMode_Horizonal,
  WaterUIStackMode_Layered,
} WaterUIStackMode;

typedef enum WaterUIToggleStyle {
  WaterUIToggleStyle_Default,
  WaterUIToggleStyle_CheckBox,
  WaterUIToggleStyle_Switch,
} WaterUIToggleStyle;

typedef struct WaterUIComputedStr {
  uintptr_t inner[2];
} WaterUIComputedStr;

typedef struct WaterUIComputedBool {
  uintptr_t inner[2];
} WaterUIComputedBool;

typedef struct WaterUIText {
  struct WaterUIComputedStr content;
  struct WaterUIComputedBool selection;
} WaterUIText;

typedef struct WaterUIAnyView {
  uintptr_t inner[2];
} WaterUIAnyView;

typedef struct WaterUITypeId {
  uint64_t inner[2];
} WaterUITypeId;

typedef struct WaterUIAction {
  uintptr_t inner[2];
} WaterUIAction;

typedef struct WaterUIButton {
  struct WaterUIAnyView label;
  struct WaterUIAction action;
} WaterUIButton;

typedef struct WaterUIViews {
  struct WaterUIAnyView *head;
  uintptr_t len;
} WaterUIViews;

typedef struct WaterUIStack {
  struct WaterUIViews views;
  enum WaterUIStackMode mode;
} WaterUIStack;

typedef struct WaterUIBindingStr {
  uintptr_t inner[1];
} WaterUIBindingStr;

typedef struct WaterUITextField {
  struct WaterUIAnyView label;
  struct WaterUIBindingStr value;
  struct WaterUIComputedStr prompt;
} WaterUITextField;

typedef struct WaterUIBindingBool {
  uintptr_t inner[1];
} WaterUIBindingBool;

typedef struct WaterUIToggle {
  struct WaterUIAnyView label;
  struct WaterUIBindingBool toggle;
  enum WaterUIToggleStyle style;
} WaterUIToggle;

typedef struct WaterUIComputedInt {
  uintptr_t inner[2];
} WaterUIComputedInt;

typedef struct WaterUIProgress {
  struct WaterUIAnyView label;
  struct WaterUIComputedInt progress;
  enum WaterUIProgressStyle style;
} WaterUIProgress;

typedef struct WaterUIBindingInt {
  uintptr_t inner[1];
} WaterUIBindingInt;

typedef struct WaterUIStepper {
  struct WaterUIBindingInt value;
  struct WaterUIComputedInt step;
} WaterUIStepper;

typedef struct WaterUIOnceErrorViewBuilder {
  uintptr_t inner[2];
} WaterUIOnceErrorViewBuilder;

typedef struct WaterUIRemoteImage {
  struct WaterUIComputedStr url;
  struct WaterUIAnyView loading;
  struct WaterUIOnceErrorViewBuilder error;
} WaterUIRemoteImage;

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
  struct WaterUIAnyView content;
  struct WaterUIPadding value;
} WaterUIWithValue_Padding;

typedef struct WaterUIBridge {
  uintptr_t inner[1];
} WaterUIBridge;

typedef struct WaterUIClosure {
  void *data;
  void (*call)(const void*);
  void (*free)(void*);
} WaterUIClosure;

typedef struct WaterUIEnvironment {
  uintptr_t inner[1];
} WaterUIEnvironment;

typedef struct WaterUIUtf8Data {
  uint8_t *head;
  uintptr_t len;
} WaterUIUtf8Data;

typedef struct WaterUIComputedData {
  uintptr_t inner[2];
} WaterUIComputedData;

typedef struct WaterUIData {
  uint8_t *head;
  uintptr_t len;
} WaterUIData;

typedef struct WaterUIComputedView {
  uintptr_t inner[2];
} WaterUIComputedView;

typedef struct WaterUIError {
  struct WaterUIUtf8Data msg;
} WaterUIError;

typedef struct WaterUIErrorViewBuilder {
  uintptr_t inner[2];
} WaterUIErrorViewBuilder;

typedef struct WaterUIApp {
  struct WaterUIAnyView content;
  struct WaterUIEnvironment env;
} WaterUIApp;

typedef struct WaterUIAppClosure {
  void *data;
  void (*call)(const void*, struct WaterUIApp);
  void (*free)(void*);
} WaterUIAppClosure;

struct WaterUIText waterui_view_force_as_text(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_text_id(void);

struct WaterUIButton waterui_view_force_as_button(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_button_id(void);

struct WaterUIStack waterui_view_force_as_stack(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_stack_id(void);

struct WaterUITextField waterui_view_force_as_field(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_field_id(void);

struct WaterUIToggle waterui_view_force_as_toggle(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_toggle_id(void);

struct WaterUIProgress waterui_view_force_as_progress(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_progress_id(void);

struct WaterUIStepper waterui_view_force_as_stepper(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_stepper_id(void);

struct WaterUIRemoteImage waterui_view_force_as_remoteimg(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_remoteimg_id(void);

struct WaterUIWithValue_Padding waterui_modifier_force_as_padding(struct WaterUIAnyView view);

struct WaterUITypeId waterui_modifier_padding_id(void);

void waterui_drop_bridge(struct WaterUIBridge value);

int8_t waterui_send_to_bridge(const struct WaterUIBridge *bridge, struct WaterUIClosure f);

struct WaterUIBridge waterui_create_bridge(struct WaterUIEnvironment *env);

struct WaterUIBridge waterui_clone_bridge(const struct WaterUIBridge *pointer);

struct WaterUITypeId waterui_view_id(const struct WaterUIAnyView *view);

struct WaterUIAnyView waterui_call_view(struct WaterUIAnyView view, struct WaterUIEnvironment env);

struct WaterUITypeId waterui_view_empty_id(void);

void waterui_drop_binding_str(struct WaterUIBindingStr value);

struct WaterUIUtf8Data waterui_read_binding_str(const struct WaterUIBindingStr *binding);

void waterui_write_binding_str(const struct WaterUIBindingStr *binding,
                               struct WaterUIUtf8Data value);

uintptr_t waterui_subscribe_binding_str(const struct WaterUIBindingStr *binding,
                                        struct WaterUIClosure subscriber);

void waterui_unsubscribe_binding_str(const struct WaterUIBindingStr *binding, uintptr_t id);

void waterui_drop_binding_int(struct WaterUIBindingInt value);

intptr_t waterui_read_binding_int(const struct WaterUIBindingInt *binding);

void waterui_write_binding_int(const struct WaterUIBindingInt *binding, intptr_t value);

uintptr_t waterui_subscribe_binding_int(const struct WaterUIBindingInt *binding,
                                        struct WaterUIClosure subscriber);

void waterui_unsubscribe_binding_int(const struct WaterUIBindingInt *binding, uintptr_t id);

void waterui_drop_binding_bool(struct WaterUIBindingBool value);

bool waterui_read_binding_bool(const struct WaterUIBindingBool *binding);

void waterui_write_binding_bool(const struct WaterUIBindingBool *binding, bool value);

uintptr_t waterui_subscribe_binding_bool(const struct WaterUIBindingBool *binding,
                                         struct WaterUIClosure subscriber);

void waterui_unsubscribe_binding_bool(const struct WaterUIBindingBool *binding, uintptr_t id);

void waterui_drop_computed_data(struct WaterUIComputedData value);

struct WaterUIData waterui_read_computed_data(const struct WaterUIComputedData *computed);

uintptr_t waterui_subscribe_computed_data(const struct WaterUIComputedData *computed,
                                          struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_data(const struct WaterUIComputedData *computed, uintptr_t id);

void waterui_drop_computed_str(struct WaterUIComputedStr value);

struct WaterUIUtf8Data waterui_read_computed_str(const struct WaterUIComputedStr *computed);

uintptr_t waterui_subscribe_computed_str(const struct WaterUIComputedStr *computed,
                                         struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_str(const struct WaterUIComputedStr *computed, uintptr_t id);

void waterui_drop_computed_int(struct WaterUIComputedInt value);

intptr_t waterui_read_computed_int(const struct WaterUIComputedInt *computed);

uintptr_t waterui_subscribe_computed_int(const struct WaterUIComputedInt *computed,
                                         struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_int(const struct WaterUIComputedInt *computed, uintptr_t id);

void waterui_drop_computed_bool(struct WaterUIComputedBool value);

bool waterui_read_computed_bool(const struct WaterUIComputedBool *computed);

uintptr_t waterui_subscribe_computed_bool(const struct WaterUIComputedBool *computed,
                                          struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_bool(const struct WaterUIComputedBool *computed, uintptr_t id);

void waterui_drop_computed_view(struct WaterUIComputedView value);

struct WaterUIAnyView waterui_read_computed_view(const struct WaterUIComputedView *computed);

uintptr_t waterui_subscribe_computed_view(const struct WaterUIComputedView *computed,
                                          struct WaterUIClosure subscriber);

void waterui_unsubscribe_computed_view(const struct WaterUIComputedView *computed, uintptr_t id);

struct WaterUIComputedView waterui_view_force_as_computed(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_computed_id(void);

struct WaterUIError waterui_error(struct WaterUIUtf8Data msg);

struct WaterUIAnyView waterui_build_error_view(struct WaterUIError error,
                                               const struct WaterUIErrorViewBuilder *builder);

struct WaterUIAnyView waterui_build_once_error_view(struct WaterUIError error,
                                                    struct WaterUIOnceErrorViewBuilder builder);

void waterui_drop_anyview(struct WaterUIAnyView value);

struct WaterUIAnyView waterui_view_force_as_any(struct WaterUIAnyView view);

struct WaterUITypeId waterui_view_any_id(void);

void waterui_drop_env(struct WaterUIEnvironment value);

void waterui_drop_action(struct WaterUIAction value);

void waterui_call_action(const struct WaterUIAction *action, const struct WaterUIEnvironment *env);

struct WaterUIEnvironment waterui_clone_env(const struct WaterUIEnvironment *pointer);
