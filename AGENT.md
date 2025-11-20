# WaterUI CLI Agent Workflow

Use this playbook whenever you need to create a demo app with the `water`
binary and run it on the Android emulator/simulator.

## 1. Install or Update the CLI

1. From the repo root install the CLI so `water` is on `PATH`:
   ```bash
   cargo install --path ./cli --force
   ```
   Re-run this after modifying the CLI so the binary you call matches the
   sources you are editing. Make sure `~/.cargo/bin` is on `PATH` and confirm
   with `water --version`.
2. When working on the CLI itself you can also run via `cargo run -p
   waterui-cli -- …`, but the install step avoids rebuilding for every call.

## 2. Scaffold an App

1. Decide where to place the generated project. For agent runs we keep the demo
   inside `target/debug/water-demo` so it can be blown away easily between
   sessions.
2. Create the project with the Android backend enabled. Use `--yes` to skip
   prompts and `--dev` so the template points at the local workspace crates:
   ```bash
   water create \
     --name "Water Agent Demo" \
     --bundle-identifier com.example.wateragentdemo \
     --directory target/debug/water-demo \
     --backend android \
     --yes --dev
   ```
3. All further commands (`water devices`, `water run …`) should be executed from
   the repository root so relative paths resolve correctly (`--project
   target/debug/water-demo`).

## 3. Enumerate Android Targets

1. Boot an Android emulator (Android Studio or `emulator -avd <name>`). Physical
   devices work as well once USB debugging is enabled.
2. List runtimes. The CLI implements `water devices`; some docs mention
   `water devices list`, but the bare command is the one that exists:
   ```bash
   water devices --format json
   ```
   Use `--format json` in automation and pick the Android `name` or `id` you
   want to run against (e.g., `Pixel_8_Pro_API_35`). Without JSON, the CLI
   prints a table you can scan manually.
3. Always capture the device identifier explicitly—non-interactive runs fail
   if `--device` is omitted.

## 4. Run on the Android Emulator

1. Export the backend checkout so the CLI can point Gradle at the local
   artifacts:
   ```bash
   export WATERUI_ANDROID_DEV_BACKEND_DIR=$PWD/backends/android
   ```
   (Prefix the command inline when scripting.)
2. Invoke the Android runner with the project path and exact device name:
   ```bash
   WATERUI_ANDROID_DEV_BACKEND_DIR=backends/android \
     water run android \
       --project target/debug/water-demo \
       --device Pixel_8_Pro_API_35
   ```
   Older notes may say `water android run`; the current syntax is `water run
   android`.
3. The CLI performs a Rust build, syncs JNI libs into Gradle, then drives the
   emulator. Hot reload stays active until you exit; add `--no-watch` for CI.
4. Use `--format json` or `--json` for machine-readable progress. Pass
   `--release` once you need optimized artifacts.

## 5. Troubleshooting Checklist

1. **Device discovery fails** → rerun `water devices --format json` with `-vv`
   for logs and ensure the emulator is booted.
2. **Build errors inside `waterui` crates** → fix Rust sources in this repo, run
   `cargo update` inside `target/debug/water-demo`, reinstall the CLI, then
   rerun step 4.
3. **CLI behavior drifts** → edit `/cli`, run tests if needed, and reinstall via
   `cargo install --path ./cli --force` before invoking `water` again.
4. **Android backend drift** → patch `/backends/android`, then rerun the Android
   command with `WATERUI_ANDROID_DEV_BACKEND_DIR=backends/android` pointing at
   your updated checkout.
5. **Cross-repo development etiquette**
   - Always apply changes directly in this WaterUI workspace first, then
     `git commit` + `git push` to the `dev` branch of this repository.
   - Update submodules (`backends/android`, `backends/apple`) in their own repos
     on their `dev` branches as needed; commit + push before syncing here.
   - After pushing upstream changes, run `water backend update` (to pull fresh
     backend artifacts) or `cargo update -p waterui -p waterui-ffi` inside your
     generated app so it tracks the newly published commits.
   - Keep this checklist handy so you don’t forget the `dev`-branch-first rule.
