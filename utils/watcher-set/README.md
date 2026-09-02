# waterui-watcher-set

A removable watcher registry: register a callback, get a guard back, and drop
the guard to unregister.

Event sources that keep a plain `Vec` of callbacks have no way to remove one, so
a watcher registered by a short-lived observer stays registered for the lifetime
of the source — and whatever that watcher captured stays alive with it. This
crate owns that bookkeeping once, for every WaterUI component that publishes
events to more than one listener.

```rust
use waterui_watcher_set::WatcherSet;

let set = WatcherSet::<u32>::new();
let guard = set.insert(|value| tracing::debug!(value, "event"));
set.emit(&7);
drop(guard); // the watcher is gone
```

The registry is single-threaded (`Rc` + `RefCell`), like the rest of WaterUI's
UI layer, and is `no_std` with `alloc`.

Part of the [WaterUI](https://github.com/water-rs/waterui) framework.
