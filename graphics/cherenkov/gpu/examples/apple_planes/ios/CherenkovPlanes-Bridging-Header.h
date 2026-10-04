#ifndef CherenkovPlanes_Bridging_Header_h
#define CherenkovPlanes_Bridging_Header_h

// Exported by `libapple_planes.a` (see `cherenkov_planes_start` in the
// Rust crate). `view` is the UIView the engine presents into — kept
// alive by its window for the run's life. `width`/`height` are the
// view's size in points, `scale` the display scale. All calls run on
// the main thread; a rejected launch aborts the process with a message
// in os_log.
void cherenkov_planes_start(const void *view, double width, double height, double scale);
#include <stdbool.h>
bool cherenkov_planes_tick(void);
bool cherenkov_planes_finished(void);
void cherenkov_planes_resize(double width, double height, double scale);

// The run dims the panel to its minimum for the measurement and hands
// the brightness back on resign-active, termination and every fatal
// path; the app re-dims when it returns to the foreground.
void cherenkov_planes_brightness_restore(void);
void cherenkov_planes_brightness_dim(void);

#endif /* CherenkovPlanes_Bridging_Header_h */
