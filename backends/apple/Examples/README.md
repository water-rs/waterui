# Apple-specific examples

Examples here exercise capabilities only the Apple backend has — Liquid Glass
surfaces and controls, the iOS 26 tab bar behaviors — and so live beside the
backend rather than in the framework's cross-platform `examples/`.

Each example is a WaterUI playground project in the same shape as a framework
example (`waterui.workspace = true`, `waterui_path = "../.."`). It is meant to be
staged into a framework checkout's `examples/` directory, where it becomes a
workspace member and builds against that checkout with this backend spliced in:

```bash
cp -R Examples/liquid_glass "$WATERUI_DIR/examples/liquid_glass"
cd "$WATERUI_DIR/examples/liquid_glass" && water run --platform ios
```

`.github/scripts/setup-e2e.sh` does that staging for CI, so the examples run in
the same shards, with the same baselines and SwiftUI twins, as the framework's.
