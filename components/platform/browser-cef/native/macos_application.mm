#import <Cocoa/Cocoa.h>
#import <dlfcn.h>

#include "include/cef_application_mac.h"

namespace {

struct WaterUICefSandboxSymbol {
  void *library;
  void *address;
};

struct WaterUICefSandboxContext {
  void *library;
  void *context;
};

WaterUICefSandboxSymbol LoadSandboxSymbol(const char *symbol) {
  @autoreleasepool {
    NSURL *directory =
        [NSURL fileURLWithPath:[[NSBundle mainBundle] executablePath]]
            .URLByDeletingLastPathComponent;
    NSFileManager *file_manager = [NSFileManager defaultManager];
    while (directory.path.length > 1) {
      NSURL *runtime_manifest =
          [directory URLByAppendingPathComponent:
                         @"Resources/waterui-browser/cef/runtime.json"];
      if ([file_manager fileExistsAtPath:runtime_manifest.path]) {
        NSURL *library_url = [directory
            URLByAppendingPathComponent:@"Frameworks/Chromium Embedded "
                                         "Framework.framework/Libraries/"
                                         "libcef_sandbox.dylib"];
        void *library = dlopen(library_url.fileSystemRepresentation,
                               RTLD_NOW | RTLD_LOCAL | RTLD_FIRST);
        if (library == nullptr) {
          @throw [NSException
              exceptionWithName:@"WaterUICEFInitializationError"
                         reason:[NSString
                                    stringWithFormat:
                                        @"Failed to load CEF sandbox at %@: %s",
                                        library_url.path, dlerror()]
                       userInfo:nil];
        }
        void *address = dlsym(library, symbol);
        if (address == nullptr) {
          @throw [NSException
              exceptionWithName:@"WaterUICEFInitializationError"
                         reason:[NSString
                                    stringWithFormat:
                                        @"CEF sandbox at %@ does not export %s",
                                        library_url.path, symbol]
                       userInfo:nil];
        }
        return {library, address};
      }
      directory = directory.URLByDeletingLastPathComponent;
    }
    @throw [NSException
        exceptionWithName:@"WaterUICEFInitializationError"
                   reason:@"CEF executable is not inside a packaged WaterUI "
                           "application runtime"
                 userInfo:nil];
  }
}

} // namespace

extern "C" void *cef_sandbox_initialize(int argc, char **argv) {
  WaterUICefSandboxSymbol symbol = LoadSandboxSymbol("cef_sandbox_initialize");
  auto initialize = reinterpret_cast<void *(*)(int, char **)>(symbol.address);
  void *context = initialize(argc, argv);
  if (context == nullptr) {
    dlclose(symbol.library);
    return nullptr;
  }
  return new WaterUICefSandboxContext{symbol.library, context};
}

extern "C" void cef_sandbox_destroy(void *context) {
  auto *sandbox = static_cast<WaterUICefSandboxContext *>(context);
  void *address = dlsym(sandbox->library, "cef_sandbox_destroy");
  if (address == nullptr) {
    @throw [NSException
        exceptionWithName:@"WaterUICEFInitializationError"
                   reason:[NSString
                              stringWithFormat:@"CEF sandbox does not export "
                                                "cef_sandbox_destroy: %s",
                                               dlerror()]
                 userInfo:nil];
  }
  auto destroy = reinterpret_cast<void (*)(void *)>(address);
  destroy(sandbox->context);
  dlclose(sandbox->library);
  delete sandbox;
}

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

- (void)sendEvent:(NSEvent *)event {
  CefScopedSendingEvent sendingEventScoper;
  [super sendEvent:event];
}

@end

extern "C" int waterui_cef_initialize_macos_application() {
  [WaterUICefApplication sharedApplication];
  return [NSApp isKindOfClass:[WaterUICefApplication class]];
}

extern "C" int waterui_cef_macos_application_is_active() {
  return NSApp != nil && [NSApp isKindOfClass:[WaterUICefApplication class]] &&
         [NSApp conformsToProtocol:@protocol(CefAppProtocol)];
}
