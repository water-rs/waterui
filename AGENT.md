# WaterUI Debugging Workflow

Use this checklist whenever you need to debug `water run ios` or `water run android`:

1. **Always enumerate devices first.** Run `water devices` in the demo directory before every `water run …` so the CLI can auto-select a target.
2. **Run the target platform:** execute `water run ios` (or `android`) from `target/debug/water-demo`.
3. **If compilation fails inside the root `waterui` crate:** fix the Rust sources here, then `git commit` and `git push` to `dev`. Afterwards run `cargo update` inside `target/debug/water-demo` to keep the generated app in sync.
4. **If the failure is caused by the Apple backend:** make the fix inside `/backends/apple` (git submodule), commit and push to its `dev` branch, then from the demo run `water update backend swift` to pull the new Swift artifacts.
5. **If the CLI misbehaves (prompts, device handling, etc.):** patch `/cli`, `git commit` + `git push` to `dev`, and run `cargo install --path ./cli` so subsequent invocations pick up the fix.
6. Repeat steps 1–5 until `water run ios`/`android` completes successfully.
