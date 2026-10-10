# waterui-testing-artifacts

The canonical artifact store `WaterUI` test suites write into: a
`suite/case/stage.png` layout rooted at `WATERUI_TEST_ARTIFACTS_DIR` (or the
platform temp directory), the `TestArtifacts` path helper, and the `Snapshot`
/`CapturedSnapshot` pixel records.

The store carries no renderer or harness dependency, so a backend's own
`cargo test --lib` target can name it without dragging the backend's rlib
into the link a second time. `waterui-testing` re-exports everything here.
