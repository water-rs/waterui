//
//  water.h
//  tour
//
//  Created by Lexo Liu on 29/11/2023.
//

#ifndef water_h
#define water_h
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

typedef enum WaterUIAlignment {
  WaterUIAlignment_Default,
  WaterUIAlignment_Leading,
  WaterUIAlignment_Center,
  WaterUIAlignment_Trailing,
} WaterUIAlignment;

typedef enum WaterUIStackMode {
  WaterUIStackMode_Vertical,
  WaterUIStackMode_Horizonal,
} WaterUIStackMode;

typedef struct WaterUIEventObject {
  uintptr_t inner[2];
} WaterUIEventObject;

typedef struct WaterUIBuf {
  uint8_t *head;
  uintptr_t len;
  uintptr_t capacity;
} WaterUIBuf;

typedef struct WaterUIViewObject {
  uintptr_t inner[2];
} WaterUIViewObject;

typedef struct WaterUIText {
  struct WaterUIBuf buf;
  bool selectable;
} WaterUIText;

typedef struct WaterUIButton {
  struct WaterUIViewObject label;
  struct WaterUIEventObject action;
} WaterUIButton;

typedef struct WaterUITapGesture {
  struct WaterUIViewObject view;
  struct WaterUIEventObject event;
} WaterUITapGesture;

typedef struct WaterUIAction {
  struct WaterUIBuf label;
  struct WaterUIEventObject action;
} WaterUIAction;

typedef struct WaterUIActions {
  struct WaterUIAction *head;
  uintptr_t len;
  uintptr_t capacity;
} WaterUIActions;

typedef struct WaterUIMenu {
  struct WaterUIViewObject label;
  struct WaterUIActions actions;
} WaterUIMenu;

typedef struct WaterUITextField {
  struct WaterUIBuf label;
  const void *value;
  struct WaterUIBuf prompt;
} WaterUITextField;

typedef enum WaterUISize_Tag {
  WaterUISize_Default,
  WaterUISize_Size,
} WaterUISize_Tag;

typedef struct WaterUISize {
  WaterUISize_Tag tag;
  union {
    struct {
      double size;
    };
  };
} WaterUISize;

typedef struct WaterUIEdge {
  struct WaterUISize top;
  struct WaterUISize right;
  struct WaterUISize bottom;
  struct WaterUISize left;
} WaterUIEdge;

typedef struct WaterUIFrame {
  struct WaterUISize width;
  struct WaterUISize min_width;
  struct WaterUISize max_width;
  struct WaterUISize height;
  struct WaterUISize min_height;
  struct WaterUISize max_height;
  struct WaterUIEdge margin;
  enum WaterUIAlignment alignment;
} WaterUIFrame;

typedef struct WaterUIFrameModifier {
  struct WaterUIFrame frame;
  struct WaterUIViewObject view;
} WaterUIFrameModifier;

typedef struct WaterUIViews {
  struct WaterUIViewObject *head;
  uintptr_t len;
  uintptr_t capacity;
} WaterUIViews;

typedef struct WaterUIStack {
  enum WaterUIStackMode mode;
  struct WaterUIViews contents;
} WaterUIStack;

typedef struct WaterUISubscriberObject {
  const void *state;
  void (*subscriber)(const void*);
} WaterUISubscriberObject;

typedef struct WaterUISubscriberBuilderObject {
  const void *state;
  struct WaterUISubscriberObject (*subscriber)(const void*);
} WaterUISubscriberBuilderObject;

/**
 * # Safety
 * `EventObject` must be valid
 */
void waterui_call_event_object(struct WaterUIEventObject object);

/**
 * # Safety
 * `Binding` must be valid
 */
void waterui_drop_string_binding(const void *binding);

/**
 * # Safety
 * `Binding` must be valid, and `Buf` is valid UTF-8 string.
 */
void waterui_set_string_binding(const void *binding, struct WaterUIBuf string);

/**
 * # Safety
 * `Binding` must be valid.
 */
struct WaterUIBuf waterui_get_string_binding(const void *binding);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_empty(struct WaterUIViewObject view);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_text(struct WaterUIViewObject view, struct WaterUIText *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_button(struct WaterUIViewObject view, struct WaterUIButton *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_tap_gesture(struct WaterUIViewObject view, struct WaterUITapGesture *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_menu(struct WaterUIViewObject view, struct WaterUIMenu *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_text_field(struct WaterUIViewObject view, struct WaterUITextField *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_frame_modifier(struct WaterUIViewObject view,
                                      struct WaterUIFrameModifier *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
int8_t waterui_view_to_stack(struct WaterUIViewObject view, struct WaterUIStack *value);

/**
 * # Safety
 * `EventObject` must be valid
 */
struct WaterUIViewObject waterui_call_view(struct WaterUIViewObject view);

/**
 * # Safety
 * `EventObject` must be valid
 */
void waterui_add_subscriber(struct WaterUIViewObject view,
                            struct WaterUISubscriberBuilderObject subscriber);

extern uintptr_t waterui_create_window(struct WaterUIBuf title, struct WaterUIViewObject content);

extern void waterui_window_closeable(uintptr_t id, bool is);

extern void waterui_close_window(uintptr_t id);

extern struct WaterUIViewObject waterui_main(void);

#endif /* water_h */
