# Release Brief

Version draft prepared for product and platform teams.

## What Landed

- Full tree-sitter incremental parsing in `flow_markdown`.
- Token entrance animation with configurable fade-in.
- Better handling for incomplete markdown edges in stream mode.

## QA Checklist

- [x] Append-only stream path validated.
- [x] Full markdown load path validated.
- [ ] Long document soak test on low-power devices.

## Compatibility Matrix

| Platform | Status | Notes |
| --- | --- | --- |
| macOS | Ready | `water run --platform macos` |
| iOS Simulator | In progress | needs hardware decode validation |
| Android | Planned | pending codec parity |

See [WaterUI](https://waterui.dev) for project documentation.
