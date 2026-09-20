# Changelog

All notable changes to `waterui-internal` are documented in this file.

## [Unreleased]

## [0.5.0](https://github.com/water-rs/waterui/compare/waterui-internal-v0.4.1...waterui-internal-v0.5.0) - 2026-09-20

### Added

- *(theme)* add the Error and ErrorForeground colour slots
- *(view)* add ViewExt::muted() for the muted-foreground token
- *(style)* carry the shadow caster's corner radius in Shadow
- *(webview)* include_web! mounts a web frontend through the asset origin
- *(assets)* [**breaking**] include_bundle! generates its mount and reports it through artifact metadata
- *(a11y)* [**breaking**] add a value channel to accessibility nodes
- *(text)* add a monospaced font design as a semantic slot

### Fixed

- *(accordion)* publish the header as a single activatable button
- gate facade asset re-exports by target like the assets runtime
- *(when)* resolve plain bool conditions statically
- *(layout)* draw Divider as a hairline in the Border token colour
- *(clippy)* scope install() const expectation to Apple targets ([#987](https://github.com/water-rs/waterui/pull/987))
- *(testing)* install the self-drawn video realization on every host
- *(layout)* declare the View bound on every view-taking generic
- *(cli)* install the app's translation catalog at every generated boundary

### Other

- Merge pull request #1028 from water-rs/fix/948-error-token
- Merge remote-tracking branch 'origin/dev' into feat/edge-insets-sugar
- Merge remote-tracking branch 'origin/dev' into feat/view-muted
- Merge pull request #974 from water-rs/fix/test-video-realization
- *(layout)* [**breaking**] drop the SafeAreaInsets contract
- Merge pull request #814 from water-rs/feat/preview-debug-gate
- Merge pull request #829 from water-rs/fix/list-stretch-axis
- Merge pull request #775 from water-rs/feat/glass-background
- Merge pull request #799 from water-rs/fix/video-gpu-opt-in
- Merge pull request #781 from water-rs/feat/link-target
- Merge pull request #776 from water-rs/fix/must-use-query-window
- *(focus)* cover Focused binding semantics and runtime ui_focus arbitration
- reorder suiteki imports ahead of waterui_*
- merge dev into refactor/suiteki-switch
- [**breaking**] replace waterui-str with the extracted suiteki crate
- Merge pull request #635 from water-rs/feat/a11y-value-channel-457
- *(deps)* move the requirements the extraction left behind ([#557](https://github.com/water-rs/waterui/pull/557))

## [0.4.1](https://github.com/water-rs/waterui/compare/waterui-internal-v0.4.0...waterui-internal-v0.4.1) - 2026-09-11

### Added

- *(media)* let a photo preserve its aspect ratio inside the frame it is given ([#542](https://github.com/water-rs/waterui/pull/542))

## [0.4.0](https://github.com/water-rs/waterui/compare/waterui-internal-v0.3.0...waterui-internal-v0.4.0) - 2026-09-11

### Added

- *(text)* define the default type scale once
- *(waterui)* expose standalone components through the facade
- *(math)* speak the formula instead of announcing its MathML
- *(markdown)* make fenced code claimable and route FlowMarkdown through Code
- *(rich-text)* typeset Markdown math instead of dropping it

### Fixed

- *(web)* restore the wasm32 build of the hydrolysis web runner and check it in CI
- *(waterui-internal)* hoist a test helper above the statements it follows
- *(markdown)* make FlowMarkdownConfig a constant signal
- *(animation)* [**breaking**] merge the duplicate AnimationExt into one system-default trait
- *(release)* close package rehearsal gaps
- *(release)* verify registry-only package graph

### Other

- *(deps)* take the outstanding independent dependency releases
- Merge pull request #428 from water-rs/agent/kit-dialog-wasm-clippy-413
- *(wasm)* lint the wasm32 lane instead of only compiling it
- Merge pull request #275 from water-rs/agent/clippy-all-features-scope/20260902-131826
- *(text)* move Code into waterui-text so components can claim fences without depending on the root crate
- [**breaking**] depend on the self-drawn component crates directly
- [**breaking**] put syntect, pulldown-cmark, ICU formatters, bcrypt and regex behind features
- *(deps)* delete dependencies nothing imports
- [**breaking**] let the composition root install self-drawn realizations
- ship the licence texts in every published crate
- prepare WaterUI 0.3 release versions

## [0.3.0] - 2026-08-25

- Aligned the facade implementation and feature graph with the WaterUI 0.3 public release cohort.
