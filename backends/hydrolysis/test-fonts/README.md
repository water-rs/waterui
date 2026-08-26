# Deterministic test fonts

Roboto, from the release the `water` CLI's font registry ships to applications
(<https://github.com/googlefonts/roboto/releases/download/v2.138/roboto-android.zip>),
licensed under the Apache License 2.0 (see `LICENSE` beside the files).

Headless test hosts shape text against these fonts instead of whatever the
host OS discovers, so a layout assertion tuned on one platform's system fonts
holds on every other, and snapshot goldens are identical across macOS, Linux,
and Windows runners. Characters outside Roboto's coverage deliberately shape
as missing glyphs — a test that needs another script should say so loudly
rather than silently depending on the host's fallback set.

Production rendering is untouched: applications keep the resource fonts the
CLI stages next to the executable, and the system fallback chain behind them.
