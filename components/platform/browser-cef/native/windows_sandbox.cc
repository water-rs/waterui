#include "include/cef_sandbox_win.h"

extern "C" void* waterui_cef_windows_sandbox_create() {
  return new CefScopedSandboxInfo();
}

extern "C" void* waterui_cef_windows_sandbox_info(void* sandbox) {
  return static_cast<CefScopedSandboxInfo*>(sandbox)->sandbox_info();
}

extern "C" void waterui_cef_windows_sandbox_destroy(void* sandbox) {
  delete static_cast<CefScopedSandboxInfo*>(sandbox);
}
