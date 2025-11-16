# AGENT NOTES

- The `cli/src/terminal` tree is just the presentation shell for the WaterUI CLI. Keep all platform/device logic, toolchain handling, and build/runtime behavior inside the `waterui_cli` library modules under `cli/src` (e.g., `device`, `platform`, `project`).
- Interactive UX bits (prompt wording, spinner animations, printf helpers) belong in `cli/src/terminal`, but they should only orchestrate calls into the library and render results.
- When adding new behavior, first extend the library APIs (e.g., expose helpers in `device/android.rs`) and then have the terminal frontend call into them. Avoid duplicating logic between the frontend and the library.
- Respect the user's preference for readable console output: use the helpers in `cli/src/terminal/ui.rs` for styling/spinners so logs remain consistent, and keep the JSON mode quiet.
- If you need to capture build logs or device diagnostics, hook into the library-level abstractions (e.g., `AndroidDevice`) so both the CLI frontend and other consumers benefit.
- Never edit the generated `waterui.h` headers by hand. Regenerate them with `cargo run -p waterui-ffi --bin generate_header --features cbindgen`, which writes `ffi/waterui.h` and then copies it into `backends/apple/Sources/CWaterUI/include/waterui.h` and `backends/android/runtime/src/main/cpp/waterui.h`. The Android backend lives in its own repository, so keeping the copies in sync via the generator is essential for CI.
