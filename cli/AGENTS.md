# CLI AGENTS.md

The `water` CLI (crate `waterui-cli`) orchestrates creating, building, previewing, running and packaging WaterUI apps across platforms. The rules in the root [`AGENTS.md`](../AGENTS.md) apply to every file here too.

It is a workspace member with its own release line: `cli/v*` tags and the `cli-dist`/`cli-release-assets` legs of `release.yml`. It resolves its framework crates as workspace path dependencies in the one lockfile and learns everything else about the framework at run time, so a change to a CLI↔framework contract lands in one commit. This repository owns both sides of that contract: `[package.metadata.waterui]` in the root manifest (`minimum-cli-version`, backend coordinates, the Android API floor, `scaffold-packages`), the `framework.json` the release attaches, the `waterui_meta_*` records the macros emit, and `cli/` itself. A fact the CLI cannot obtain from the framework is a framework issue; a CLI defect is a CLI issue; both live here.

## Commands

| Command | Does |
| --- | --- |
| `water create` / `water init` | scaffold a project / adopt the current directory |
| `water channel` | inspect or explicitly update the project's framework channel |
| `water build --platform <platform>` | build for a platform and backend |
| `water run` | build and deploy to a device or simulator |
| `water package` | package the release build; `--debug` packages an unoptimized one |
| `water preview` | render a `#[preview]` view to PNG |
| `water bench` | run `#[waterui::bench]` benchmarks and collect reports |
| `water mcp` | serve the app to an agent over MCP |
| `water doctor` / `water devices` / `water device` | check the environment / list and manage devices |
| `water clean` / `water gc` / `water fetch` | clean artifacts / collect stale caches / fetch declared fonts |

## Rules

- **Read only designed channels.** The CLI locates previews, tests, benchmarks, assets and capabilities through compiler artifacts (`waterui_meta_*` records in the project crate's library artifact), the manifests it owns (`Water.toml`), and `cargo metadata` — never `.rs` files (root rule on source scraping). When no channel carries a fact, build it in the framework.
- **The terminal layer is thin.** `src/terminal/` parses arguments and formats output through `Shell`; every decision — resolution, device selection, build orchestration, doctor checks — lives in the library and is unit-testable without a terminal.
- **Every probe of the machine goes through `Host`** (`src/toolchain/host.rs`): PATH lookup, environment variables, process spawning. `Host::current()` is the real machine; tests build a `Host` over a scratch PATH of fake tools (`src/toolchain/testing.rs`, `fake_tools.{sh,cmd}`) and an explicit environment map. No `std::env::var`, `which::which`, or `Command::new` outside the host seam, and no environment mutation in tests.
- **Fail fast, no fallbacks.** An unresolvable channel, a missing manifest key, a toolchain reporting the wrong shape — each is an error naming what was found and what was required, never a silent substitute. `doctor` classifies a problem as fixable or not; it does not paper over it.
- Diagnostics go through `tracing` (`RUST_LOG=debug water …` prints to stderr; `--logs` on `water run` is the *device* log level). Structured text is serialized (`serde`) or rendered from a typed `askama` template under `src/templates/`, never concatenated.
- `tests/doctor_json.rs` runs the built binary with `doctor --json` and checks identity and schema of every item, never status, so it stays deterministic on any machine.
- Tests that build framework source or hit the network are `#[ignore]`d with the reason and run only in the nightly `cli-ignored` job (`--run-ignored ignored-only`); the per-PR gate passes offline on a clean runner. The nightly `cli-e2e` legs run `doctor --fix`, `create`, `build`, `package` and `run` as a fresh OS user on macOS, Windows and Linux against this checkout — a failure there is a bug, not flakiness to retry.

## Framework channels and version selection

Framework channels are not Rust toolchain channels. The CLI supports:

| Channel | Source and guarantee |
| --- | --- |
| `dev` | The integration `dev` branch, resolved to an exact commit; compilation-checked, not test-certified. |
| `nightly` | An immutable tagged revision whose complete required integration suite passed, with the tested dependency/backend combination. It is not a moving development branch or merely the latest commit of the day. |
| `stable` | Formal crates.io releases with compatible native-backend versions; publishing remains owned by release-plz. Scaffold packages whose workspace requirement is a git pin are experimental: the stable `framework.json` omits them from `scaffold` and records them under `experimental-packages`. |

- **The CLI owns coherent version selection.** Resolve a channel on project creation or an explicit version change or update, then persist the channel and exact resolved versions. Normal build and run operations never silently upgrade dependencies or mix a tested framework with independently advancing backend branches. Nightly keeps its tested dependency lock rather than resolving newer compatible component releases; stable uses published crates; local-checkout development stays explicit and never overrides a requested channel implicitly. With no certified nightly, return an error, never fall back to dev. Resolution lives in the library, not in terminal wrappers.
- **The minimum CLI version is enforced before scaffolding or building.** The library checks the requirement in local source metadata, in the resolved `waterui` package metadata, and persisted with a channel. Rejection names the installed and required versions and gives an update command for the selected source; it never silently switches channels or upgrades the executable. Previously released CLIs without this check need a one-time manual update.

## Preview

`#[preview]` marks a view function for rendering without running the app; its exported symbol is `waterui_preview_{crate_name}_{fn_name}`, the crate name avoiding conflicts. `water preview` builds and runs a generated preview binary in-process and the platform's backend renders the view to PNG:

- `--platform macos` / `--platform ios`: a generated Apple preview binary (`waterui-apple/preview`) — AppKit on macOS, UIKit through Mac Catalyst for iOS, no simulator.
- `--platform android`: the Hydrolysis preview host's instrumentation on a connected device, over `adb`.
- `--platform linux` / `--platform windows`: a generated Hydrolysis preview binary.
