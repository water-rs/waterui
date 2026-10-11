# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.3](https://github.com/water-rs/cli/compare/v0.4.2...v0.4.3) - 2026-09-21

### Added

- add water fetch to seed the asset cache ([#144](https://github.com/water-rs/cli/pull/144))

### Fixed

- *(macos)* give backend binaries an rpath that reaches the staged runtime ([#142](https://github.com/water-rs/cli/pull/142))
- *(ci)* give the nightly preview leg the theme Hydrolysis requires ([#139](https://github.com/water-rs/cli/pull/139))
- drop the font-manifest probe that scan_fonts replaced ([#141](https://github.com/water-rs/cli/pull/141))
- *(preview)* call a pump the Hydrolysis runtime still has ([#138](https://github.com/water-rs/cli/pull/138))
- *(assets)* make font declarations explicit and resolve fonts without network access ([#134](https://github.com/water-rs/cli/pull/134))
- scan the built crate's manifest for permissions and features ([#137](https://github.com/water-rs/cli/pull/137))
- *(ci)* make the MSRV job build with the declared floor ([#136](https://github.com/water-rs/cli/pull/136))
- doctor detects MSVC build tools and dxc, fixes winget-less hosts ([#135](https://github.com/water-rs/cli/pull/135))
- run macOS app bundles directly so launch crashes surface ([#132](https://github.com/water-rs/cli/pull/132))
- first-run polish — checksum message, update source, single Project::open, target/package output ([#129](https://github.com/water-rs/cli/pull/129))
- recover a shared dylib unit without dep-info and clean project units from the shared target ([#128](https://github.com/water-rs/cli/pull/128))
- *(create)* keep video behind an opt-in `media` feature in the scaffold ([#130](https://github.com/water-rs/cli/pull/130))
- *(doctor)* check the host's backends outside a project and group items by backend ([#120](https://github.com/water-rs/cli/pull/120))
- initialise only the managed backends the command's platform needs ([#119](https://github.com/water-rs/cli/pull/119))
- stage the shared runtime Cargo reported, never a directory probe ([#121](https://github.com/water-rs/cli/pull/121))

### Other

- lint on every OS, and fix the finding that hid behind not doing it ([#131](https://github.com/water-rs/cli/pull/131))

## [0.4.2](https://github.com/water-rs/cli/compare/v0.4.1...v0.4.2) - 2026-09-20

### Fixed

- resolve the stable channel from the registry, not the GitHub API ([#113](https://github.com/water-rs/cli/pull/113))
- let the managed crate add packages the project never locked ([#109](https://github.com/water-rs/cli/pull/109))
- *(release)* link Linux binaries against glibc 2.28 and embed archive checksums ([#107](https://github.com/water-rs/cli/pull/107))

## [0.4.1](https://github.com/water-rs/cli/compare/v0.4.0...v0.4.1) - 2026-09-20

### Added

- add `water update` for receipt-installed binaries ([#98](https://github.com/water-rs/cli/pull/98))

## [0.4.0](https://github.com/water-rs/cli/compare/v0.3.2...v0.4.0) - 2026-09-20

### Added

- [**breaking**] hydrolysis scaffolds, preview and mcp construct the runtime with Material3::defaults() ([#100](https://github.com/water-rs/cli/pull/100))
- add `water completions` for shell completion scripts

### Fixed

- *(android)* honour [backends.android] backend_path as a local runtime source ([#102](https://github.com/water-rs/cli/pull/102))
- *(package)* enable wasm-opt feature flags for rustc's wasm32 output ([#96](https://github.com/water-rs/cli/pull/96))
- *(preview)* build hydrolysis preview env through the app's composition root ([#94](https://github.com/water-rs/cli/pull/94))
- *(build)* unwind without LTO when a release development build links the shared runtime
- *(apple)* adopt the UIScene life cycle in the iOS scaffold

### Other

- Merge branch 'dev' into fix/project-toolchain-for-generated-crates
- format the plist manifest assertion
- *(release)* install shell completions in the Homebrew formula

## [0.3.2](https://github.com/water-rs/cli/compare/v0.3.1...v0.3.2) - 2026-09-17

### Added

- gate scaffolded backends on the channel's distributable packages
- add experimental WinUI backend support ([#45](https://github.com/water-rs/cli/pull/45))
- require confirmation for experimental backends ([#44](https://github.com/water-rs/cli/pull/44))

### Fixed

- *(ci)* stream fresh-user e2e output and timestamp phases
- *(build)* default water build to the backend's development profile
- *(build)* find fresh dylib dep-info on cargo's build-dir layout and skip proc-macro units ([#71](https://github.com/water-rs/cli/pull/71))
- *(apple)* forward build-script link-search paths into OTHER_LDFLAGS ([#70](https://github.com/water-rs/cli/pull/70))
- repair nightly fresh-user and ignored-test legs ([#65](https://github.com/water-rs/cli/pull/65))
- *(android)* build the preview runtime's libstd from source at 16 KB alignment ([#66](https://github.com/water-rs/cli/pull/66))
- *(preview)* keep pacing while in-flight work publishes progressive frames
- *(preview)* hold the virtual clock while settling and stop on live producers
- *(preview)* let real I/O finish before the hydrolysis preview captures
- *(build)* keep a shared-runtime source that already sits in the stage directory
- *(ci)* repair the nightly legs
- canonicalize pinned checkout paths with dunce
- *(esp32)* compare patch tables in the emitter's precedence
- *(esp32)* flag manifests missing the framework patch tables stale
- *(esp32)* carry the framework patch tables into the generated manifest
- *(framework)* resolve declared git sources for scaffold packages
- *(framework)* honor declared Apple backend revisions ([#32](https://github.com/water-rs/cli/pull/32))
- *(ci)* keep publish/dist reachable on dispatch ([#29](https://github.com/water-rs/cli/pull/29))

### Other

- *(nightly)* give the fresh-user run a cold-build budget and a WARP adapter on Windows
- *(terminal)* share the clap TargetBackend enum between commands
- format create.rs
- Merge remote-tracking branch 'origin/dev' into feat/experimental-scaffold-packages
- Merge pull request #55 from water-rs/fix/892-preview-settle-real-time
- *(apple)* remove dead simctl sample fixture and ignore .vscode
- restore the trailing newline rustfmt requires

## [0.3.1](https://github.com/water-rs/cli/compare/v0.3.0...v0.3.1) - 2026-09-15

### Fixed

- *(ci)* publish and dist legs only run on workflow_dispatch ([#27](https://github.com/water-rs/cli/pull/27))

## [0.3.0](https://github.com/water-rs/cli/compare/v0.2.1...v0.3.0) - 2026-09-15

### Added

- *(framework)* read android-backend-revision from the manifest
- *(run)* add build profiles for optimization and debug info
- *(cli)* remember the last-used run device per target
- *(cli)* run on physical iOS devices via devicectl
- *(host)* trace every spawn through the host seam
- *(cli)* water run serves include_web! mounts from the dev server
- *(cli)* brand the scaffolded web starter with WaterUI
- *(cli)* water orchestrates a declared web frontend
- *(webview)* include_web! mounts a web frontend through the asset origin
- *(assets)* [**breaking**] include_bundle! generates its mount and reports it through artifact metadata
- *(cli)* project the launch screen onto the Android SplashScreen API
- *(cli)* project the launch screen onto iOS through UILaunchScreen
- *(cli)* feature-gate esp32, svg-icons, and preview subsystems
- *(cli)* experimental TUI backend behind water run --tui
- *(cli)* [**breaking**] scaffold the backends at the release tag of the CLI's own version
- *(text)* add a monospaced font design as a semantic slot
- *(gtk)* clip to custom paths on a GTK 4.14 floor
- *(cli)* select coherent dev, nightly and stable framework channels
- *(gpu-surface)* give a self-drawn surface an accessibility label
- *(cli)* offer STIX Two Math in the built-in font registry
- *(navigation)* carry the destination's transition to native backends
- *(safe-area)* pad the window's own overlay layers by the device insets
- *(theme)* [**breaking**] add the SelectionContainer / SelectionForeground tokens
- *(cli)* name disabled default features as Swift conditions
- *(text)* [**breaking**] line_limit — SwiftUI's lineLimit on every backend
- *(cli)* [**breaking**] add water bench and remove water preview perf
- *(navigation)* links carry an ignorable marker; Apple gets its own snackbar
- *(cli)* launcher icons for self-drawn desktop targets
- *(cli)* render adaptive app icons from one vector source
- add locale-aware CJK and RTL support
- *(inspector)* offer to inspect an element from the application itself
- *(cli)* subset bundled esp32 fonts to configured unicode ranges
- *(cli)* esp32 fonts config, ESP32-P4 target, and a firmware opt-level fix
- *(cli)* warn when a dependency needs a permission the app has not enabled
- *(ffi)* [**breaking**] gate map behind an off-by-default feature
- *(inspector)* make the inspector a real WaterUI app
- [**breaking**] close the issue #134-#148 audit sweep
- unify label semantics and menu runtimes
- *(video)* continue player parity work
- *(cli)* add hydrolysis web backend and preview pipeline
- *(cli)* tighten backend-platform policy and doctor checks
- *(cli)* strengthen linux doctor toolchain detection and auto-fix
- continue deep review fixes and ffi fast-fail cleanup
- ship gpu on-demand rendering, animation updates, and cli/runtime sync
- integrate assets, media, and runtime updates
- *(cli)* add confirmed recursive clean mode
- *(preview)* Add water preview command for view rendering
- Support keyboard command on macOS
- Implement Audio Visualizer with Microphone Input and CLI Permission Support
- *(icons)* Refactor icon system with extensible CLI architecture
- Introduce `waterui-canvas`, `waterui-shape`, and `waterui-svg` crates, refactor graphics components, and integrate Vello for advanced rendering.
- enhance font management and color handling across platforms
- font bundling system and download icons at build time
- enhance error reporting in command execution and add target triple retrieval
- add Kotlin and Java support to Android toolchain and diagnostics
- Add support for streaming native platform logs in run options and log streaming
- add macOS local device gesture support and screenshot functionality
- Implement screenshot capture functionality for iOS and Android devices
- *(android)* enhance log streaming to capture panic information
- Enhance panic handling and logging in macOS device management
- Enhance macOS crash detection and logging
- *(cli)* implement crash debugging output and enhance logging
- *(shape)* Introduce shape system for view clipping and filling
- add GTK4 Color and LazyContainer components, and update dependencies for GPU support
- Introduce TargetPlatform enum and migrate backends
- add GTK backend support to WaterUI CLI
- Enhance window management and hot reload functionality
- *(media-picker)* add media picker example with photo, video, and live photo selection
- Add mac screenshot image and improve markdown example
- *(cli)* enable hot reload by default for water run
- Enhance panic logging and error handling in Apple backends
- enhance logging levels for Apple platforms and improve app structure
- *(examples)* add gesture and list examples with corresponding templates and assets
- *(android)* streamline APK installation process and add CMake toolchain wrapper for ABI support
- *(android)* switch from ComponentActivity to AppCompatActivity and disable Compose
- *(android)* implement multi-ABI packaging support and optimize build configurations
- *(android)* add support for multiple architectures and clean jniLibs
- Rewrite CLI
- Add project creation with git initialization and new playground example
- Update dependencies and enhance backend configurations
- Implement hot reload server and connection handling
- Enhance Apple platform support with new platform kinds and SDKs
- Add initial Android and Apple project templates
- enhance Android and Apple platform support with new configurations and utility functions
- update platform implementations for Android and Apple, improve function signatures and add architecture handling
- enhance platform support and add new build options for Android and Apple
- enhance Android and Apple device support with new functionalities
- enhance Android and Apple platform support, add Homebrew toolchain manager
- *(android)* refactor backend and platform modules
- Enhance async support and device scanning in backend implementations
- Refactor toolchain error handling and enhance hot reload configuration
- Implement report generation for various command results
- Introduce async runtime and enhance file watcher functionality
- Revamp README and enhance Android hot reload functionality
- Enhance video component functionality and interrupt handling
- Enhance Android packaging and hot reload functionality
- Introduce permission management for playground projects
- Add CLAUDE.md for project guidance and build instructions
- Implement interruptible command execution and secure metadata handling
- Enhance panic reporting and logging in hot reload system
- Introduce comprehensive logging and panic reporting plan
- Enhance debugging and layout system in Apple backend
- Implement safe area handling in layout system
- Enhance local development mode for WaterUI
- Rename Android library to libwaterui_app.so
- add Gemini CLI assistant documentation and subagent manager script
- add Git commit hash to build output and update dependencies
- enhance Dockerfile and documentation for improved build and configuration
- enhance Android platform support with target triples and improve artifact stripping
- integrate hyper and tungstenite for hot reload server functionality
- *(run)* implement hot reload support and ensure cdylib generation
- *(android)* enhance NDK toolchain checks and environment configuration
- *(logging)* add log filter support for hot reload and CLI
- *(logging)* enhance tracing and panic forwarding for improved log management
- *(hot_reload)* implement hot reload support for FFI and backend configurations
- update dependencies to use git sources and improve error handling in CLI
- *(android)* enhance Java environment detection for macOS by including Android Studio's bundled JBR
- add debugging workflow checklist and enhance non-interactive terminal handling
- Add watcher functionality for AnyViews and related types
- *(apple)* enhance simulator boot logic and add state checking
- *(device)* add platform filtering for device listing
- document hot reload support for Android, Apple, Web, and TUI backends
- add TUI platform support and enhance hot reload functionality
- *(hot-reload)* enhance hot reload configuration and update related commands
- Introduce build command for native artifacts
- *(cli)* enhance Android backend integration with improved logging and automation
- enhance Android backend integration and logging capabilities
- *(cli)* add backend list command
- *(cli)* support backend upgrade with ffi checks
- *(cli)* add backend management command
- *(cli)* use local android backend in dev
- Update Android project structure and dependencies for improved backend integration
- Implement hot reload functionality with configurable environment
- *(cli)* enhance JSON output support across commands
- update build process to emit stable version tags for waterui and swift backend; enhance dependency resolution with branch support
- rename swift backend to apple backend and update related configurations
- update release workflow to remove swift and android backend support; add dynamic repo URL for Swift backend
- Add compiler options to suppress Kotlin version compatibility check for Compose (now app can be compiled but still not launched)
- Update Compose compiler extension version to 1.5.14 in build.gradle.kts and template
- Add Jetpack Compose support and update MainActivity for Compose integration
- Enhance Android project setup by adding sanitized crate name and copying libc++_shared.so
- Add Android target configurations and enhance toolchain setup for Rust builds
- Enhance Android and watchOS support with new SDK checks and build scripts
- Add Android tool management and build functionality; refactor device handling
- Migrate from anyhow to color_eyre for error handling and add build script for version management
- *(android)* Enhance project creation and packaging
- Add web backend support with asset creation and configuration
- Implement gesture handling in WaterUI with WuiGestureView and GestureSequenceContainer
- Add sccache and mold configuration for improved build performance
- Enhance typography and UI components with new font styles and improved view handling
- Add Xcode project management functions and improve macOS support in run module
- Implement cleanup command to remove build artifacts and platform caches
- Add fix option to WaterUI Doctor for automatic issue resolution
- Implement WaterUI Doctor for environment checks and add new dependencies
- Introduce WuiFixedContainer for fixed layout management
- Add support for multiple backends (Swift and Android) in project configuration and creation
- Add waterui-color workspace and integrate color resolution
- add project creation functionality with templates for Android and SwiftUI and remove gtk4 backend
- add CLI package with initial command structure and dependencies
- *(cli)* add WaterUI CLI for project management

### Fixed

- *(framework)* pin the Apple backend to the framework's channel ([#20](https://github.com/water-rs/cli/pull/20))
- *(framework)* stop emitting same-source patch tables for dev channels ([#18](https://github.com/water-rs/cli/pull/18))
- *(apple)* read simulator logs inside the device, not from host logd ([#11](https://github.com/water-rs/cli/pull/11))
- *(apple)* replay simulator log history after stream attach ([#6](https://github.com/water-rs/cli/pull/6))
- *(web)* stop the dev-server tree on Windows through a job object
- *(host)* keep a daemon a child launches from holding water's caller's pipe
- run CI on main, require one framework pin, cover cmake install on every OS
- *(cli)* canonicalize the manifest path before cargo metadata
- *(cli)* kill the dev-server process group, not just the pm child
- *(apple)* read build-script outputs in nightly's nested build/ layout ([#902](https://github.com/water-rs/cli/pull/902))
- *(cli)* locate shared WaterUI runtime under both Cargo artifact layouts ([#891](https://github.com/water-rs/cli/pull/891))
- *(cli)* restore the waterui_path/backends/apple fallback
- *(cli)* collect apple archives without a Swift bridge
- *(cli)* resolve the app closure manifest on backends without an FFI companion
- *(cli)* collect cargo:rustc-link-* flags from every crate's build output
- *(graphics)* strip wgpu debug flags on Android emulators
- *(cli)* embed the Android NDK version in the binary
- *(cli)* keep the staged waterui_assets tree whole inside Apple bundles
- *(cli)* reject iOS simulators whose runtime is below the deployment target before building
- *(cli)* refuse a bundle identifier Android cannot use at create time ([#783](https://github.com/water-rs/cli/pull/783))
- *(cli)* satisfy clippy in tui backend plumbing
- *(cli)* carry the backend submodule pins in the scaffold metadata
- *(cli)* shape the support app's build cache before writing into it ([#569](https://github.com/water-rs/cli/pull/569))
- *(cli)* say which preview app answered instead of blaming the TCP server ([#565](https://github.com/water-rs/cli/pull/565))
- *(cli)* write local [patch] tables where Cargo reads them ([#567](https://github.com/water-rs/cli/pull/567))
- *(cli)* hold the scaffold versions to the workspace, and correct them ([#554](https://github.com/water-rs/cli/pull/554))
- *(cli)* pin the scaffold backend commits to the current submodules
- *(cli)* pin the run and package context futures on the heap
- *(cli)* pin the preview dylib build future on the heap
- *(cli)* carry the checkout's [patch] tables into a local-path project
- *(cli)* write framework patches as explicit tables
- *(android)* clip a scroll container to its own viewport
- *(android)* keep a gesture with the view that took its touch-down
- *(cli)* use dunce canonicalize and stage hydrolysis assets before build on Windows
- *(cli)* check native library versions in the Linux doctor
- *(cli)* report the diagnostics a failed command's tail elides
- *(apple)* attach the native capture fence's completion handler before committing it
- *(cli)* repin the backend scaffold commits onto the recorded submodules
- *(cli)* repin backend scaffold commits after the GpuFrame scale bump
- *(cli)* version waterui-core and hydrolysis-m3 from their own manifests
- *(release)* close package rehearsal gaps
- *(release)* make WPE runtime assets reproducible
- complete cross-platform acceptance gaps
- *(android)* unblock Pixel example execution
- *(picker)* name the control after its own label on GTK and Apple
- *(android)* honour the shape kind, and inset each split column
- *(android)* stop shipping a 4 KB-aligned Rust runtime
- *(android)* let an arc continue the subpath it was given
- *(cli)* repair the Windows-only code the new lint step found
- *(cli)* compare manifest paths in the form cargo reports them
- *(cli)* subset the font this repository ships, not one the host might have
- *(ffi)* unwrap the JNI surface handle before rendering offscreen
- *(ffi)* commit the regenerated Apple header copy
- *(android)* draw list section headers between the card groups
- *(cli)* derive every capability decision from the crate that provides it
- *(cli)* repin the apple backend to the current submodule
- *(cli)* make every macOS preview attempt self-sufficient
- *(cli)* repin the backend commits to the current submodules
- *(cli)* resolve the bench report directory before handing it to nextest
- *(cli)* gate the macOS icns chain to macOS
- *(cli)* inherit workspace [patch] in generated backend crates and refresh backend pins
- *(cli)* compile the copy_file import on every host
- *(hydrolysis)* share one headless executor per thread and drain it on drop
- *(cli)* repin the backend commits to the current submodules
- *(ffi)* restore the Android build broken by the navigation-link marker
- *(cli)* point the apple scaffold pin at the backend commit dev builds
- *(cli)* point the apple scaffold pin at the backend commit dev actually builds
- *(cli)* discard both halves of a support app built against another checkout
- *(inspector)* open the endpoint only when somebody asks for it
- *(preview)* discard a support app built against a different WaterUI
- *(preview)* scaffold a preview module its workspace can actually resolve
- *(cli)* re-pin the backends at the commits the workspace now uses
- *(preview)* resolve a preview module and its runtime together
- *(cli)* stop the inspector's discovery bailing from expression position
- *(cli)* pin the backends at the commits the workspace uses
- *(cli)* name an Apple application after its project
- *(inspector)* make "Inspect Element" open an inspector, and say why when it cannot
- *(cli)* skip running apps with unreadable bundles during launch
- *(cli)* carry the dev profile into every generated scaffold
- *(cli)* give hand-assembled macOS bundles their app icon
- *(inspector)* show the node the application asked to inspect
- *(cli)* give the inspector support app the feature its FFI scaffold requires
- *(runtime)* start every application, and let "inspect element" reach an inspector
- *(map)* pinch zoom and committed pans on the surface-gesture path
- *(cli)* fingerprint the Apple shared runtime with its real feature set
- *(ffi)* enable the facade inspector feature the init path requires
- *(cli)* fingerprint the Android shared runtime with its real feature set
- *(cli)* playground esp32 device config + esp32-safe app scaffold
- *(cli)* repin the scaffold backends to the current submodules
- *(cli)* stop one killed sweep from disabling build-cache GC forever
- *(android)* restore the GPU surface that the gpu feature gate removed
- *(cli)* surface backend pin drift where the person who caused it will see it
- *(cli)* make every generated-crate build agree on incremental compilation
- *(cli)* repin the Android backend fallback ref to the submodule commit
- *(cli)* condense runtime fingerprints into the shared target directory name
- *(cli)* invalidate generated host crates when scaffold templates change
- *(preview)* capture GPU content and render offscreen at 2x
- fix repository rule violations and refresh documentation
- *(cli)* synchronize Android backend pin
- *(cli)* pin repaired Apple backend
- *(cli)* convert generated slider labels
- *(cli)* generate semantic slider labels
- *(preview)* resolve local ABI dependencies from wrappers
- *(preview)* unify host and module dependency graphs
- *(preview)* match shared runtime features
- *(preview)* share the macOS WaterUI runtime
- *(preview)* scope dynamic linking to macOS
- fix macos playground quit on window close
- fix macos app stale dock process cleanup
- fix cli application exit classification
- fix hydrolysis m3 picker interactions
- fix playground cache layout and cli cache bugs
- fix cli review regressions
- repair post-rebase video integration
- *(cli)* strengthen GTK pkg-config doctor checks
- *(preview)* harden symbol resolution and render error handling
- *(android)* keep emulator alive and prefer Vulkan shared context
- *(android)* bundle libc++_shared for Rust JNI apps
- *(log)* Change log streaming level to default and increase timeout duration
- enhance CLI launch command handling and update environment variable documentation
- update android backend dependency version to 0.2.0
- update minimum version for Swift package reference to 0.2.0
- update repository URLs in template files for Apple and Android backends
- update README and Cargo.toml files to specify README.md for all components
- update waterui version to 0.2 in templates and documentation
- format function parameters for better readability in tests
- update dependencies and improve path handling in template context
- improve code formatting and readability across multiple files
- correct spelling errors and improve comments across the codebase
- update llvm-strip command to preserve dynamic symbol table
- improve interrupt handling in wait_for_interrupt function
- remove hot reload import for non-WASM targets
- clean up imports and improve command output messages in Android and Apple device modules
- *(android)* add INTERNET permission to AndroidManifest for network access
- *(dependencies)* replace log with tracing for improved logging consistency
- update run functions to handle no_watch parameter for improved behavior
- Update RendererViewComponent to use rawPtr for memory management consistency

### Other

- *(deps)* bump the pinned waterui revision to fc9aecb0
- scaffold the web template tests against the pinned framework
- name RUST_LOG as the CLI's own diagnostic switch
- stage the web_meta fixture against the pinned framework
- pin the framework at waterui dev 8bf8b7fce
- assert patch paths by value and make the Windows fixtures byte-stable
- chain the release build off release-plz's report instead of a PAT-pushed tag
- add standalone CI, nightly coverage, and release plumbing
- *(cli)* replace framework-checkout reads with pinned-revision fixtures
- *(cli)* resolve the framework repository from the manifest pin
- make waterui-cli a standalone crate
- make the pre-decoupling fixture version-agnostic
- *(apple)* pin the backend at 0.3.0-dev.1 for the webview asset origin
- *(cli)* rustfmt the fallback test
- Merge pull request #853 from water-rs/feat/run-env-flag
- Merge pull request #816 from water-rs/feat/webview-asset-origin
- Merge pull request #846 from water-rs/feat/ffi-capability-gating
- Merge pull request #657 from water-rs/fix/apple-package-link-flags
- Merge pull request #815 from water-rs/feat/preview-symbol-discovery
- Merge pull request #835 from water-rs/fix/apple-release-strip
- Merge pull request #849 from water-rs/fix/emulator-spirv-debug
- Merge remote-tracking branch 'origin/dev' into fix/ndk-version-embedded
- Merge pull request #787 from water-rs/refactor/android-min-api-metadata
- Merge pull request #780 from water-rs/feat/stable-channel-manifest
- Merge pull request #825 from water-rs/fix/clean-test-bundle-id
- Merge pull request #823 from water-rs/feat/mcp-animation-stepping
- Merge pull request #818 from water-rs/fix/apple-explicit-assets-folder
- Merge pull request #799 from water-rs/fix/video-gpu-opt-in
- Merge pull request #774 from water-rs/fix/android-artifact-metadata
- Merge pull request #754 from water-rs/fix/cli-sigterm-orphans
- Merge pull request #771 from water-rs/fix/preview-catalog
- Merge pull request #793 from water-rs/fix/android-r8
- Merge pull request #790 from water-rs/fix/generated-release-profile
- Merge pull request #789 from water-rs/fix/libcxx-needed-only
- Merge pull request #782 from water-rs/refactor/cli-typed-errors
- Merge pull request #717 from water-rs/feat/launch-config
- patch extracted crates' git-source waterui deps onto the resolved framework ([#761](https://github.com/water-rs/cli/pull/761))
- resolve path-pinned hydrolysis deps from the checkout's own sources ([#747](https://github.com/water-rs/cli/pull/747))
- water mcp serves a preview tool that returns PNG image content ([#704](https://github.com/water-rs/cli/pull/704))
- Merge branch 'dev' into feat/cli-feature-gates
- *(cli)* apply rustfmt ordering to feature-gated code
- Merge pull request #706 from water-rs/feat/cli-tui-backend
- *(backends)* extract waterui-gtk to water-rs/gtk-backend ([#646](https://github.com/water-rs/cli/pull/646))
- *(backends)* extract waterui-dew to water-rs/dew
- merge main (0.4.1 release commits) back into dev
- *(cli)* scaffold against the Apple backend commit the tree pins ([#570](https://github.com/water-rs/cli/pull/570))
- *(deps)* move the requirements the extraction left behind ([#557](https://github.com/water-rs/cli/pull/557))
- *(backends)* consume hydrolysis 0.2.0 from crates.io and drop the in-tree copy ([#553](https://github.com/water-rs/cli/pull/553))
- *(backends)* consume hydrolysis-m3 0.2.0 from crates.io and drop the in-tree copy ([#550](https://github.com/water-rs/cli/pull/550))
- Merge remote-tracking branch 'gh/main' into agent/merge-release-0.4
- release
- *(backends)* pin the backend dev heads with the slider measurement fix
- Merge pull request #512 from water-rs/agent/graphics-deps-405
- *(deps)* vello_cpu 0.2 and the remaining tooling bumps
- *(backends)* pin the merged backend dev heads
- *(android)* take the runtime's type-resolved detekt pass
- Merge pull request #404 from water-rs/agent/cef-generic-input-252
- Merge pull request #378 from water-rs/fix/windows-run-canonicalize-icon
- Merge pull request #384 from water-rs/agent/apple-inspect-node-326
- Merge pull request #385 from water-rs/agent/doctor-linux-lib-versions-376
- *(cli)* pin the scaffold to the Android runtime with the bounded colour fill
- *(apple)* take apple-backend dev at dd07ae94 and move the CLI pin with it
- *(apple)* take apple-backend dev at aa55c057 and move the CLI pin with it
- Merge pull request #346 from water-rs/agent/failure-report-diagnostics
- *(apple)* pin apple-backend a676865 and demo the text-field selection menu
- Merge pull request #309 from water-rs/agent/apple-capture-fence
- Merge pull request #263 from water-rs/agent/watcher-set-crate/20260902-105630
- *(webview)* decouple the Chromium CDP session from waterui-webview
- *(cli)* move the built-in font registry into TOML data
- repin the submodules and CLI scaffold refs onto the rewritten histories
- [**breaking**] depend on the self-drawn component crates directly
- *(deps)* turn off default features nothing in the workspace uses
- *(deps)* drop `image`'s AVIF encoder from every consumer
- [**breaking**] select the WebView engine in the application, not the backend
- [**breaking**] let the composition root install self-drawn realizations
- ship the licence texts in every published crate
- prepare WaterUI 0.3 release versions
- prepare publishable dependency graph
- *(ffi)* regenerate the C header after the doc-comment cleanup
- clear the rustdoc warnings
- *(gtk)* bound the split destination cache
- Revert the GPU throttle: concurrency is not the lever
- *(cli)* repin the android backend to the redirect fix
- *(cli)* repin the android backend to the split-selection fix
- compile the graphics, macros and cli READMEs too
- *(cli)* repin the android backend to the split-pane fix
- *(cli)* repin the android backend to the offscreen-image commit
- *(cli)* repin the android backend to the tab-icon commit
- *(cli)* repin the backend submodule commits after the header regen
- *(android)* pick up the navigation chrome fixes in the Android backend
- Give the scroll region its content, and the rest of dev's red CI
- Format the workspace
- pick up the sidebar material fix
- *(cli)* move the backend pins onto the new submodule commits
- *(cli)* share one generated-backend target dir across backends
- resolve lints in the new inspector and text code
- *(cli)* follow the backends this branch advanced
- *(video)* [**breaking**] rename the video AspectRatio mode to ContentMode
- *(cli)* pin the Android backend at the HDR opt-in commit
- *(cli)* flash-budget the esp32 release profile
- bump backend submodules for the header preamble sync and repin the CLI
- clear workspace lint stragglers and repin backend submodule refs
- *(cli)* repin backend fallback refs to the current submodule commits
- *(apple)* link development builds against a shared library, not a 428MB archive
- *(cli)* repin backend fallback refs to the current submodule commits
- *(cli)* assert the preview wrapper's crate type structurally
- *(cli)* move preview perf reporting out of the terminal layer
- *(cli)* build one crate type, cache what matters, keep artifacts across CLI upgrades
- Give every unsafe a reason the compiler can check, workspace-wide
- Point the CLI's fallback backend refs at the current submodules
- Fix hydrolysis preview scenario pointer events
- Fix lint regressions from the GTK cleanup
- Fix Linux-only lint failures across ffi, gtk, cli and preview
- Fix non-macOS CLI stubs
- Gate macOS-only CLI imports
- Make CEF independent of WaterUI backends
- Integrate CEF across WaterUI backends
- Fix macOS CEF bundle signing
- Add bundled WebView and Chromium runtimes
- Add production GPU vector map fallback
- Fix Android runtime and device workflows
- Fix Hydrolysis animation frame scheduling
- Fix Flow Markdown runtime and CLI exit tracking
- Fix shared Hydrolysis development cache
- upgrade workspace dependencies
- Fix platform ABI selection for dynamic builds
- Align preview host dependency resolution
- Use shared runtime for development builds
- *(cli)* isolate preview module linkage
- *(preview)* harden build and runtime lifecycle
- Upgrade Zenwave and remove unused Skyzen
- deliver modular cross-platform video player
- refactor native backends and GPU surface integration
- *(cli)* template the preview perf report with askama
- achieve zero clippy warnings across the workspace
- clean up clippy warnings across the workspace
- derive preview-support `waterui-preview` path from workspace metadata
- water CLI: ESP32-C3 (RISC-V) as a chip-architecture-aware platform
- water CLI: first-class ESP32 platform with dew backend
- Managed backends keep Cargo.lock across regeneration
- Preview API cleanup: HydrolysisPreviewRequest groups shared inputs; drop dead interaction methods
- Preview protocol: one serde run-config and JSON perf report replace env-var/k=v encodings
- managed hydrolysis backends regenerate on template-content mismatch
- Fix Hydrolysis example rendering and macOS acceptance
- Fix Hydrolysis text interaction responsiveness
- Enable Hydrolysis macOS accessibility windows
- Fix Hydrolysis Material 3 text field and ripple animations
- Optimize Hydrolysis filter redraw frames
- Add preview perf layer budgets
- Support Hydrolysis preview scroll scenarios
- Report rendered frame stats in preview perf
- Measure redraw frames in Hydrolysis preview perf
- Break down Hydrolysis preview perf rebuild phases
- Import source crate in Hydrolysis preview expressions
- Reduce Hydrolysis root scroll clip layers
- Report Hydrolysis compositor layers in preview perf
- Improve preview perf reporting
- Upgrade Hydrolysis preview perf telemetry
- Add default previews for examples and form perf
- Add Hydrolysis preview test and perf commands
- Fix Hydrolysis resize and run crash reporting
- Add Material filter chip preview support
- Import picker constructor in expression previews
- Restore Hydrolysis single preview rendering path
- Install Material snackbar theme tokens
- Refine Material preview expression and progress color timing
- Implement Hydrolysis Material motion preview foundation
- Add explicit Hydrolysis Material 3 preview support
- reorganize the project
- Clean up crate layout
- Re-export List/Section/row helpers from prelude
- List API ergonomics + AGENTS design principles + backend defaults
- Photo/Video/VideoPlayer on_event accept EventHandler
- Add ListSection semantic marker and group inspector by topic
- Redesign inspector to use NavigationView + native List rows
- Redesign inspector dashboard with stronger visual hierarchy
- Sync Android backend scaffold ref
- Sync Apple backend scaffold ref
- Verify Android host tools after doctor fixes
- Prepare dev for release automation
- Remove WaterKit fs from release crates
- Sync Apple backend scaffold ref
- Fix preview dylib platform matching
- Fix CI coverage and preview dylib targets
- Fix Android tooling and strict linting
- Clean dev CI and example builds
- Fix Android packaging toolchain resolution
- Fix Apple simulator packaging build settings
- Optimize preview wrapper linking
- expose support app render timings
- trim crate-type override and persist support logs
- trim dylib debug info and init support tracing
- move rust stdlib rpath to build and drop runtime dylib patching
- use local-path render in CLI and unify HasDylib cache truth
- use local dylib path and extend support-app idle lifetime
- enforce dev dynamic-linking and split runtime wrapper crates
- Fix remaining preview support app workarounds
- Optimize preview support app launch and hot path
- Stabilize preview runtime fingerprint identity
- Align scaffold backends and versions to build refs
- Fix CLI dev checkout mode detection
- Refactor waterui-cli lint-heavy command flows
- *(cli)* type-safe template scaffolding
- Refactor CLI managed build cache
- snapshot in-progress canonical changes
- remove legacy asset pipeline
- add clean flag for global playground cache
- unify cache and support app helpers
- Remove cached font archives after extraction
- Fix recursive clean cache discovery
- Checkpoint current WaterUI changes
- Refactor asset and icon build helpers
- Merge branch 'feat/calendar-jiff' into dev
- Merge remote-tracking branch 'origin/dev' into dev
- Reapply local hydrolysis and toolchain changes after syncing dev
- Merge origin/dev into dev and resolve hydrolysis renderer conflict
- Fix hydrolysis text input/focus freeze and update Linux toolchain deps
- Fix form text-field freeze and add cross-distro Linux doctor checks
- Bump dependency versions across workspace
- checkpoint all in-progress workspace changes
- include waterui dependency in generated hydrolysis backend crates
- refresh stale playground hydrolysis scaffolding when deps drift
- implement app/playground backend orchestration
- *(cli)* remove obsolete dav1d toolchain integration
- commit pending workspace changes on dev
- enforce fast-fail rules and remove legacy fallback paths
- commit full review and inspector/runtime updates
- harden fast-fail paths and runtime compatibility
- Auto-install Meson on macOS when cargo build fails
- Fix GTK main thread executor and video renderer
- Enhance video, chart, and locale platform support
- Refactor MediaPicker and add reminders example
- *(cli)* use async fs checks in recursive clean
- Add smart asset management system with asset! macro
- Remove hot reload functionality in favor of preview system
- Add Android JNI FFI support and improve CLI process management
- Move static assets to R2 CDN and add PID-based window capture
- Enhance documentation and examples for reactive patterns in WaterUI, including new guidelines for bindings and real-time updates
- Add sccache support for improved build performance and caching
- Refactor preview app communication and remove unresponsive app handling
- Enhance preview app communication and error handling
- Refactor code for improved readability and consistency
- anyviews watch FFI + preview/gtk/window improvements
- Fix ZStack stretch behavior
- Integrate preview TCP server into view
- Add preview system and refactor core APIs
- CLI improvements and misc updates
- simplify `text!` macro calls by removing the `waterui::` prefix.
- rename GTK backend to GTK4
- release
- Release 0.2.0
- release
- improve code readability and consistency across multiple files
- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- streamline media picker and loading state management
- Refactor Native Component and Improve Error Handling
- Refactor Android device run logic and update Gradle configurations for improved build process
- Add log level management for device log streaming across platforms
- Enhance Android and Apple backends with permission management and Gradle build improvements
- Add AndroidEmulator device and defer launch to CLI
- Embed CLI commit hash and add water dir management
- Add Android SDK path helpers and use adb_path
- Switch zenwave/skyzen to crates.io versions
- Add playground backend paths and target dir support
- Update submodules and enhance macOS launcher UI
- Implement CLI commands for WaterUI: build, clean, create, devices, doctor, package, and run
- Enhance documentation and improve code clarity across multiple modules
- Refactor backend trait and implement new build system
- WTF...Let's rewrite it!
- clean up and enhance project structure and backend definitions
- Refactor CLI toolchain and installation modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Simplify WebSocket close handling and clean up code formatting
- Update workspace configuration and dependencies for examples
- Clean up hot reload session logging and initialization
- Simplify hot reload configuration for Android and Apple devices
- Update hot reload configuration and enhance dependencies
- Remove deprecated files and streamline project structure
- Streamline playground project handling and enhance build process
- Clean up project structure and enhance CLI functionality
- Update backend submodules and enhance CLI device commands
- Refactor CLI commands for device management and enhance roadmap documentation
- Update Android backend submodule and adjust padding in template
- Update Android backend submodule and modify build process
- Standardize library naming for Android and Apple backends
- Update android backend submodule and enhance clean command functionality
- Update documentation and add CMake checks for Apple builds
- Enhance WaterUI CLI documentation and add screenshot capture feature
- Fix CLI to copy libc++_shared.so to jniLibs for Android builds
- remove unused hyper and tungstenite dependencies, integrate skyzen for hot reload functionality
- simplify hot reload library path handling and extract filename logic
- prefer Android Studio JBR when Java missing
- Align Android tooling and agent workflow
- Refactor Apple backend integration and remove deprecated files
- *(build)* streamline argument handling in build script
- Enhance UI output handling and add terminal utilities for improved user experience
- Improve command construction and error handling in project build and run functions
- Refactor CLI structure and remove unused modules
- Start to refractor WaterUI CLI...
- Rename no_watch argument to no_hot_reload for clarity and update related function signatures
- Refactor FFI bindings and hot reload functionality
- *(cli)* enhance cargo build process with sccache configuration and retry logic
- Merge pull request #23 from water-rs/codex/refactor-waterui-cli-for-enhanced-functionality
- Merge pull request #22 from water-rs/codex/modify-cli-for-water-run-prompts
- Prompt for backend selection before running
- Merge branch 'main' into codex/refactor-cli-to-use-third-party-crates
- Improve CLI web dev server stability
- Merge branch 'main' into codex/add-json-output-support-for-cli
- Add JSON output mode to CLI
- Update Package.swift for waterui and adjust Swift version in backend package
- Update component imports to use waterui_core and streamline module structure
- Remove example files for FormBuilder and #[form] macro
- remove unused development team parameter and clean up code
- Update defaultValue and deinit annotations in Gesture.swift; fix string formatting in project.pbxproj.tpl
- replace all .unwrap() to .expect()
- Refactor CLI Doctor Command and Improve Error Handling
- Removed unusable backends
- Refactor and enhance the WaterUI framework
- Break change in FFI: new vtable-based array API
- remove CLI module and related files to streamline project structure
- Clean up whitespace and formatting in various files
- Bring back `waterui_ffi`

## [0.2.1](https://github.com/water-rs/waterui/compare/waterui-cli-v0.2.0...waterui-cli-v0.2.1) - 2026-09-11

### Fixed

- *(cli)* hold the scaffold versions to the workspace, and correct them ([#554](https://github.com/water-rs/waterui/pull/554))

### Other

- *(backends)* consume hydrolysis 0.2.0 from crates.io and drop the in-tree copy ([#553](https://github.com/water-rs/waterui/pull/553))
- *(backends)* consume hydrolysis-m3 0.2.0 from crates.io and drop the in-tree copy ([#550](https://github.com/water-rs/waterui/pull/550))

## [0.2.0](https://github.com/water-rs/waterui/compare/waterui-cli-v0.1.4...waterui-cli-v0.2.0) - 2026-09-11

### Added

- *(cli)* select coherent dev, nightly and stable framework channels
- *(gpu-surface)* give a self-drawn surface an accessibility label
- *(cli)* offer STIX Two Math in the built-in font registry

### Fixed

- *(cli)* pin the scaffold backend commits to the current submodules
- *(cli)* pin the run and package context futures on the heap
- *(cli)* pin the preview dylib build future on the heap
- *(cli)* carry the checkout's [patch] tables into a local-path project
- *(cli)* write framework patches as explicit tables
- *(android)* clip a scroll container to its own viewport
- *(android)* keep a gesture with the view that took its touch-down
- *(cli)* use dunce canonicalize and stage hydrolysis assets before build on Windows
- *(cli)* check native library versions in the Linux doctor
- *(cli)* report the diagnostics a failed command's tail elides
- *(apple)* attach the native capture fence's completion handler before committing it

### Other

- *(backends)* pin the backend dev heads with the slider measurement fix
- Merge pull request #512 from water-rs/agent/graphics-deps-405
- *(deps)* vello_cpu 0.2 and the remaining tooling bumps
- *(backends)* pin the merged backend dev heads
- *(android)* take the runtime's type-resolved detekt pass
- Merge pull request #404 from water-rs/agent/cef-generic-input-252
- Merge pull request #378 from water-rs/fix/windows-run-canonicalize-icon
- Merge pull request #384 from water-rs/agent/apple-inspect-node-326
- Merge pull request #385 from water-rs/agent/doctor-linux-lib-versions-376
- *(cli)* pin the scaffold to the Android runtime with the bounded colour fill
- *(apple)* take apple-backend dev at dd07ae94 and move the CLI pin with it
- *(apple)* take apple-backend dev at aa55c057 and move the CLI pin with it
- Merge pull request #346 from water-rs/agent/failure-report-diagnostics
- *(apple)* pin apple-backend a676865 and demo the text-field selection menu
- Merge pull request #309 from water-rs/agent/apple-capture-fence
- Merge pull request #263 from water-rs/agent/watcher-set-crate/20260902-105630
- *(webview)* decouple the Chromium CDP session from waterui-webview
- *(cli)* move the built-in font registry into TOML data

## [0.1.4](https://github.com/water-rs/waterui/compare/cli-v0.1.3...waterui-cli-v0.1.4) - 2026-08-25

### Changed

- Updated generated projects to WaterUI and FFI 0.3.
- Standardized runtime asset lookup on `waterui-cli-v*` release tags.

### Fixed

- Fixed WPE runtime compiler selection and release manifest lookup.

## [0.1.2](https://github.com/water-rs/waterui/compare/cli-v0.1.1...cli-v0.1.2) - 2025-12-14

### Fixed

- update android backend dependency version to 0.2.0
- update minimum version for Swift package reference to 0.2.0
- update repository URLs in template files for Apple and Android backends

## [0.1.0](https://github.com/water-rs/waterui/releases/tag/cli-v0.1.0) - 2025-12-13

### Added

- Enhance window management and hot reload functionality
- _(media-picker)_ add media picker example with photo, video, and live photo selection
- Add mac screenshot image and improve markdown example
- _(cli)_ enable hot reload by default for water run
- Enhance panic logging and error handling in Apple backends
- enhance logging levels for Apple platforms and improve app structure
- _(examples)_ add gesture and list examples with corresponding templates and assets
- _(android)_ streamline APK installation process and add CMake toolchain wrapper for ABI support
- _(android)_ switch from ComponentActivity to AppCompatActivity and disable Compose
- _(android)_ implement multi-ABI packaging support and optimize build configurations
- _(android)_ add support for multiple architectures and clean jniLibs
- Rewrite CLI
- Add project creation with git initialization and new playground example
- Update dependencies and enhance backend configurations
- Implement hot reload server and connection handling
- Enhance Apple platform support with new platform kinds and SDKs
- Add initial Android and Apple project templates
- enhance Android and Apple platform support with new configurations and utility functions
- update platform implementations for Android and Apple, improve function signatures and add architecture handling
- enhance platform support and add new build options for Android and Apple
- enhance Android and Apple device support with new functionalities
- enhance Android and Apple platform support, add Homebrew toolchain manager
- _(android)_ refactor backend and platform modules
- Enhance async support and device scanning in backend implementations
- Refactor toolchain error handling and enhance hot reload configuration
- Implement report generation for various command results
- Introduce async runtime and enhance file watcher functionality
- Revamp README and enhance Android hot reload functionality
- Enhance video component functionality and interrupt handling
- Enhance Android packaging and hot reload functionality
- Introduce permission management for playground projects
- Add CLAUDE.md for project guidance and build instructions
- Implement interruptible command execution and secure metadata handling
- Enhance panic reporting and logging in hot reload system
- Introduce comprehensive logging and panic reporting plan
- Enhance debugging and layout system in Apple backend
- Implement safe area handling in layout system
- Enhance local development mode for WaterUI
- Rename Android library to libwaterui_app.so
- add Gemini CLI assistant documentation and subagent manager script
- add Git commit hash to build output and update dependencies
- enhance Dockerfile and documentation for improved build and configuration
- enhance Android platform support with target triples and improve artifact stripping
- integrate hyper and tungstenite for hot reload server functionality
- _(run)_ implement hot reload support and ensure cdylib generation
- _(android)_ enhance NDK toolchain checks and environment configuration
- _(logging)_ add log filter support for hot reload and CLI
- _(logging)_ enhance tracing and panic forwarding for improved log management
- _(hot_reload)_ implement hot reload support for FFI and backend configurations
- update dependencies to use git sources and improve error handling in CLI
- _(android)_ enhance Java environment detection for macOS by including Android Studio's bundled JBR
- add debugging workflow checklist and enhance non-interactive terminal handling
- Add watcher functionality for AnyViews and related types
- _(apple)_ enhance simulator boot logic and add state checking
- _(device)_ add platform filtering for device listing
- document hot reload support for Android, Apple, Web, and TUI backends
- add TUI platform support and enhance hot reload functionality
- _(hot-reload)_ enhance hot reload configuration and update related commands
- Introduce build command for native artifacts
- _(cli)_ enhance Android backend integration with improved logging and automation
- enhance Android backend integration and logging capabilities
- _(cli)_ add backend list command
- _(cli)_ support backend upgrade with ffi checks
- _(cli)_ add backend management command
- _(cli)_ use local android backend in dev
- Update Android project structure and dependencies for improved backend integration
- Implement hot reload functionality with configurable environment
- _(cli)_ enhance JSON output support across commands
- update build process to emit stable version tags for waterui and swift backend; enhance dependency resolution with branch support
- rename swift backend to apple backend and update related configurations
- update release workflow to remove swift and android backend support; add dynamic repo URL for Swift backend
- Add compiler options to suppress Kotlin version compatibility check for Compose (now app can be compiled but still not launched)
- Update Compose compiler extension version to 1.5.14 in build.gradle.kts and template
- Add Jetpack Compose support and update MainActivity for Compose integration
- Enhance Android project setup by adding sanitized crate name and copying libc++\_shared.so
- Add Android target configurations and enhance toolchain setup for Rust builds
- Enhance Android and watchOS support with new SDK checks and build scripts
- Add Android tool management and build functionality; refactor device handling
- Migrate from anyhow to color_eyre for error handling and add build script for version management
- _(android)_ Enhance project creation and packaging
- Add Markdown support for rich text rendering and enhance entry point documentation
- Add web backend support with asset creation and configuration
- Implement gesture handling in WaterUI with WuiGestureView and GestureSequenceContainer
- Add sccache and mold configuration for improved build performance
- Enhance typography and UI components with new font styles and improved view handling
- Add Xcode project management functions and improve macOS support in run module
- Implement cleanup command to remove build artifacts and platform caches
- Add fix option to WaterUI Doctor for automatic issue resolution
- Implement WaterUI Doctor for environment checks and add new dependencies
- Introduce WuiFixedContainer for fixed layout management
- Add support for multiple backends (Swift and Android) in project configuration and creation
- Implement Android backend components and runtime
- Add waterui-color workspace and integrate color resolution
- add project creation functionality with templates for Android and SwiftUI and remove gtk4 backend
- add CLI package with initial command structure and dependencies
- enhance deployment workflow and add rustdoc integration; update README and introduce roadmap
- update dependencies with versioning and improve documentation clarity
- add counter example to README and enhance tutorial book with tests
- _(cli)_ add WaterUI CLI for project management

### Fixed

- format function parameters for better readability in tests
- update dependencies and improve path handling in template context
- improve code formatting and readability across multiple files
- update documentation to reflect Android View terminology for consistency
- correct spelling errors and improve comments across the codebase
- update llvm-strip command to preserve dynamic symbol table
- improve interrupt handling in wait_for_interrupt function
- remove hot reload import for non-WASM targets
- clean up imports and improve command output messages in Android and Apple device modules
- _(android)_ add INTERNET permission to AndroidManifest for network access
- _(dependencies)_ replace log with tracing for improved logging consistency
- update run functions to handle no_watch parameter for improved behavior
- _(docs)_ resolve doc test compilation errors
- Update RendererViewComponent to use rawPtr for memory management consistency

### Other

- Bump waterui version to 0.2 in documentation across multiple components
- improve code readability and consistency across multiple files
- Refactor layout tests to use approximate equality for floating-point comparisons
- Add waterui-color, waterui-str, and waterui-url crates with comprehensive documentation
- streamline media picker and loading state management
- Refactor Native Component and Improve Error Handling
- Refactor Android device run logic and update Gradle configurations for improved build process
- Add log level management for device log streaming across platforms
- Enhance Android and Apple backends with permission management and Gradle build improvements
- Add AndroidEmulator device and defer launch to CLI
- Embed CLI commit hash and add water dir management
- Add Android SDK path helpers and use adb_path
- Switch zenwave/skyzen to crates.io versions
- Add playground backend paths and target dir support
- Update submodules and enhance macOS launcher UI
- Implement CLI commands for WaterUI: build, clean, create, devices, doctor, package, and run
- Enhance documentation and improve code clarity across multiple modules
- Refactor backend trait and implement new build system
- WTF...Let's rewrite it!
- clean up and enhance project structure and backend definitions
- Refactor CLI toolchain and installation modules
- Refactor and clean up code across multiple components
- Refactor layout components and improve documentation
- Simplify WebSocket close handling and clean up code formatting
- Update workspace configuration and dependencies for examples
- Update README and FFI components for consistency and clarity
- Clean up hot reload session logging and initialization
- Simplify hot reload configuration for Android and Apple devices
- Update hot reload configuration and enhance dependencies
- Remove deprecated files and streamline project structure
- Streamline playground project handling and enhance build process
- Clean up project structure and enhance CLI functionality
- Update backend submodules and enhance CLI device commands
- Refactor CLI commands for device management and enhance roadmap documentation
- Remove AGENT.md and enhance FFI bindings for events and gestures
- Update Android backend submodule and adjust padding in template
- Update Android backend submodule and modify build process
- Standardize library naming for Android and Apple backends
- Update android backend submodule and enhance clean command functionality
- Remove terminal backend mention from README.md
- Update documentation and add CMake checks for Apple builds
- Enhance WaterUI CLI documentation and add screenshot capture feature
- Fix CLI to copy libc++\_shared.so to jniLibs for Android builds
- remove unused hyper and tungstenite dependencies, integrate skyzen for hot reload functionality
- simplify hot reload library path handling and extract filename logic
- Make Suspense/hot reload use thread-safe executor
- prefer Android Studio JBR when Java missing
- update FFI header regeneration instructions and enhance README content
- Align Android tooling and agent workflow
- Refactor Apple backend integration and remove deprecated files
- _(build)_ streamline argument handling in build script
- Enhance UI output handling and add terminal utilities for improved user experience
- Improve command construction and error handling in project build and run functions
- Refactor CLI structure and remove unused modules
- Start to refractor WaterUI CLI...
- Rename no_watch argument to no_hot_reload for clarity and update related function signatures
- Refactor FFI bindings and hot reload functionality
- _(cli)_ enhance cargo build process with sccache configuration and retry logic
- Merge pull request #23 from water-rs/codex/refactor-waterui-cli-for-enhanced-functionality
- Refactor toolchain checks and interactive doctor
- Merge branch 'main' into codex/refactor-cli-to-use-third-party-crates
- Improve CLI web dev server stability
- Merge branch 'main' into codex/add-json-output-support-for-cli
- Add JSON output mode to CLI
- Update Package.swift for waterui and adjust Swift version in backend package
- Update component imports to use waterui_core and streamline module structure
- Remove example files for FormBuilder and #[form] macro
- remove unused development team parameter and clean up code
- Update defaultValue and deinit annotations in Gesture.swift; fix string formatting in project.pbxproj.tpl
- replace all .unwrap() to .expect()
- Refactor CLI Doctor Command and Improve Error Handling
- Removed unusable backends
- Refactor layout documentation and examples for clarity and completeness
- Refactor and enhance documentation for WaterUI components
- Refactor and enhance components and utilities
- Refactor and enhance the WaterUI framework
- Refactor layout components and improve documentation
- Break change in FFI: new vtable-based array API
- update README with enhanced descriptions, quick start guide, and roadmap; improve layout section
- Update README.md
- update README to clarify framework features and demo; remove outdated sections
- Refactor documentation and examples across components for clarity and consistency
- remove CLI module and related files to streamline project structure
- Refactor dependencies in Cargo.toml to use workspace references for waterui-str and nami.
- Clean up whitespace and formatting in various files
- Bring back `waterui_ffi`
- Completely rewrite README.md with modern WaterUI patterns
- Update README files for clarity and consistency across components
- Add more lints and fix all warnings
- Refine project and Migrate to Rust 2024
- Redesign modifier and add some convenient initializer for components
- Refactor our project
- Reorganize crates, merge main crate and core crate. Better async view support
