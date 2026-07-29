#import <Cocoa/Cocoa.h>

#include "include/cef_application_mac.h"

@interface WaterUICefApplication : NSApplication <CefAppProtocol> {
 @private
  BOOL handlingSendEvent_;
}
@end

@implementation WaterUICefApplication

- (BOOL)isHandlingSendEvent {
  return handlingSendEvent_;
}

- (void)setHandlingSendEvent:(BOOL)handlingSendEvent {
  handlingSendEvent_ = handlingSendEvent;
}

- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEventScoper;
  [super sendEvent:event];
}

@end

extern "C" int waterui_cef_initialize_macos_application() {
  [WaterUICefApplication sharedApplication];
  return [NSApp isKindOfClass:[WaterUICefApplication class]];
}

extern "C" int waterui_cef_macos_application_is_active() {
  return NSApp != nil &&
         [NSApp isKindOfClass:[WaterUICefApplication class]] &&
         [NSApp conformsToProtocol:@protocol(CefAppProtocol)];
}
