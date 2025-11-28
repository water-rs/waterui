# WaterUI CLI Agent Workflow

Use this playbook whenever you need to create a demo app with the `water`
binary and run it on simulators/emulators.

## 1. Install or Update the CLI

From the repo root install the CLI so `water` is on `PATH`:

```bash
cargo install --path ./cli --force
```

Re-run this after modifying the CLI so the binary you call matches the sources
you are editing. Make sure `~/.cargo/bin` is on `PATH` and confirm with
`water --version`.

When working on the CLI itself you can also run via `cargo run -p waterui-cli
-- <command>`, but the install step avoids rebuilding for every call.

## 2. Scaffold an App

1. Decide where to place the generated project. For agent runs we keep the demo
   inside `target/debug/water-demo` so it can be blown away easily between
   sessions.
2. Create the project with your desired backends. Use `--yes` to skip prompts
   and `--dev` so the template points at the local workspace crates:
   ```bash
   water create \
     --name "Water Agent Demo" \
     --bundle-identifier com.example.wateragentdemo \
     --directory target/debug/water-demo \
     --backend android --backend swiftui \
     --yes --dev
   ```
3. All further commands (`water devices`, `water run`, etc.) should be executed
   from the repository root so relative paths resolve correctly.

## 3. List Available Devices

1. Boot a simulator or emulator (Xcode Simulator, Android Studio, or
   `emulator -avd <name>`). Physical devices work as well.
2. List available targets:
   ```bash
   water devices
   ```
   Use `water --json devices` for machine-readable output. Pick the device
   `name` or `identifier` you want to run against.

## 4. Run with Hot Reload

**Hot reload is a key feature**—always test with it enabled unless specifically
debugging non-hot-reload scenarios.

1. For Android, export the backend checkout so the CLI can point Gradle at the
   local artifacts:
   ```bash
   export WATERUI_ANDROID_DEV_BACKEND_DIR=$PWD/backends/android
   ```
2. Run the app on your target platform and device:
   ```bash
   # Android
   water run android --project target/debug/water-demo --device "emulator-5554"

   # iOS Simulator
   water run ios --project target/debug/water-demo --device "iPhone 16 Pro"

   # macOS
   water run macos --project target/debug/water-demo
   ```
3. The CLI performs a Rust build, packages the app, and launches it with hot
   reload enabled by default. Source changes trigger automatic rebuilds.
4. **Testing hot reload**: Modify `src/lib.rs` in the demo project while the app
   is running—changes should appear within seconds without restarting.
5. Use `--release` for optimized builds when profiling performance.

## 5. Capture Screenshots for Debugging

LLM agents can capture screenshots from running simulators/emulators to visually
inspect app state, verify UI changes, or debug rendering issues.

1. Ensure a simulator or emulator is running (check with `water devices`).
2. Capture a screenshot using the device name or identifier:
   ```bash
   water capture --device "emulator-5554" -o /tmp/debug-screenshot.png
   water capture --device "iPhone 16 Pro" -o /tmp/ios-screenshot.png
   ```
3. **Important**: If the device name is ambiguous (exists on multiple platforms),
   specify the platform explicitly:
   ```bash
   water capture --device "My Device" --platform android -o /tmp/screenshot.png
   water capture --device "My Device" --platform apple -o /tmp/screenshot.png
   ```
4. Use `water --json capture --device <device>` for structured output.
5. The captured PNG can be analyzed to verify UI state, detect visual
   regressions, or confirm that hot-reload changes rendered correctly.

## 6. Troubleshooting Checklist

1. **Device discovery fails** → run `water devices` with verbose logging
   (`water -vv devices`) and ensure the emulator/simulator is booted.
2. **Build errors inside `waterui` crates** → fix Rust sources in this repo,
   run `cargo update` inside `target/debug/water-demo`, reinstall the CLI, then
   rerun step 4.
3. **CLI behavior drifts** → edit `cli/`, run tests if needed, and reinstall
   via `cargo install --path ./cli --force` before invoking `water` again.
4. **Android backend drift** → patch `backends/android`, then rerun with
   `WATERUI_ANDROID_DEV_BACKEND_DIR=backends/android` pointing at your updated
   checkout.
5. **Hot reload not connecting** → check that no firewall is blocking the local
   WebSocket server. The CLI prints the port it's listening on.
6. **Cross-repo development etiquette**
   - Always apply changes directly in this WaterUI workspace first, then
     `git commit` + `git push` to the `dev` branch of this repository.
   - Update submodules (`backends/android`, `backends/apple`) in their own repos
     on their `dev` branches as needed; commit + push before syncing here.
   - After pushing upstream changes, run `water backend update` (to pull fresh
     backend artifacts) or `cargo update -p waterui -p waterui-ffi` inside your
     generated app so it tracks the newly published commits.
7. **FFI header regeneration**
   - Never edit the generated headers manually. Run
     `cargo run -p waterui-ffi --bin generate_header --features cbindgen` (see
     `ffi/generate_header.rs`) to refresh `ffi/waterui.h` plus both backend
     copies before committing/pushing changes that touch FFI symbols.

## Runtime Guardrails

- WaterUI UI code must stay on the main thread. Do **not** use
  `std::thread::spawn` in-app; schedule background work with
  `executor_core::spawn` and main-thread UI tasks with
  `executor_core::spawn_local` to avoid cross-thread panics.
