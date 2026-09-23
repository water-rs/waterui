# FFI Component Coverage Gaps

Summary of view components exported in `ffi/waterui.h` that are **not** wired up in the Apple and Android backends.

- Compared against the 29 `waterui_force_as_*` components defined in `ffi/waterui.h`.
- Apple coverage pulled from `backends/apple/Sources/WaterUI/Core/AnyView.swift`.
- Android coverage pulled from `backends/android/runtime/src/main/java/dev/waterui/android/runtime/RenderRegistry.kt`.

## Missing per backend

- Apple backend missing 9 components: `color_picker`, `link`, `list`, `list_item`, `photo`, `live_photo`, `picker`, `table`, `table_column`.
- Android backend missing 9 components: `color_picker`, `link`, `list`, `list_item`, `photo`, `live_photo`, `table`, `table_column`, `video`.

## Missing on both

- Shared gaps (8): `color_picker`, `link`, `list`, `list_item`, `photo`, `live_photo`, `table`, `table_column`.

Notes:
- Apple implements `video_player` and `video`; Android only registers `video_player` (no plain `video` renderer).
- Neither backend currently registers the list/table family or media photo/live photo components exposed by the FFI.
