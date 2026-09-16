# Contributing to WaterUI

## Issues

The issue tracker is the source of truth for work on WaterUI. If you find a problem — a bug, a broken example, a rotting workflow, a missing primitive, a design gap — file an issue before fixing it.

Each issue is **one self-contained technical task**: a single defect or a single implementable change that can be understood, assigned, and merged on its own. Do not file umbrella issues, roadmaps, or sequenced slices, and do not use "phase", "stage", or "part N of M" in titles or bodies. The one exception is a request with genuinely separable parts: file a parent issue that carries the request's intent and exists only to group its sub-issues; no implementation hangs off the parent, and each leaf lands through its own PR.

Pick the matching template under **New Issue**:

- **Bug report** — something is broken or behaves incorrectly.
- **Feature request** — a new capability, component, or API.
- **Maintenance task** — refactors, extractions, repo hygiene, CI or release plumbing.

Questions and early design ideas go to [Discussions](https://github.com/water-rs/waterui/discussions), not the issue tracker. Blank issues remain enabled for maintainers.

Titles follow `<area>: <what>` — for example `android: gradient example panics on uniform stride` or `components: row swipe actions beyond delete/move`.

Assign the earliest open version milestone to every issue, unless the work clearly belongs to a later release.

## Labels

Apply labels from each category below. Every issue needs **exactly one type label**; all other categories are optional and may be combined.

### Type (exactly one)

| Label | Meaning |
| --- | --- |
| `bug` | A defect: something is broken or behaves incorrectly. |
| `enhancement` | A new feature, component, API, or improvement. |
| `task` | Maintenance work: refactors, extractions, hygiene, plumbing. |
| `documentation` | Documentation additions or corrections. |
| `question` | A question that needs an answer before work can proceed. |
| `dependencies` | Dependency updates and version surgery. |

### Platform

Apply when the issue is specific to one backend or target. Omit for cross-platform work.

| Label | Covers |
| --- | --- |
| `android` | Android backend, JNI, Gradle packaging. |
| `apple` | iOS / macOS backend, Swift bridge, Apple packaging. |
| `gtk` | GTK backend (Linux). |
| `hydrolysis` | Hydrolysis desktop backend (WebView2 / webview). |
| `web` | Web / WASM frontend and the JS bridge. |

### Area

The part of the workspace the issue touches.

| Label | Covers |
| --- | --- |
| `cli` | The `water` CLI: scaffolding, run, package, fonts, templates. |
| `core` | `waterui-core` and the `waterui` facade: runtime, reactivity, layout, environment. |
| `components` | Component crates under `components/` (foundation, visual, multimedia, platform, …). |
| `ffi` | The C ABI layer, header generation, and per-backend FFI glue. |
| `graphics` | Rendering: `waterui-graphics`, dew, GPU surfaces, shaders, color. |
| `devtools` | Inspector, MCP server, preview tooling under `components/devtools/`. |
| `macros` | Procedural macros and code generation. |
| `testing` | Test infrastructure, snapshot baselines, example verification. |
| `ci` | GitHub Actions workflows, release plumbing, nightly gates. |

### Concerns

Cross-cutting themes. Optional, combine freely.

| Label | Meaning |
| --- | --- |
| `developer experience` | API ergonomics, error messages, onboarding, docs-as-product. |
| `compile time` | Compile and incremental build time. |
| `performance` | Runtime performance: rendering, layout, memory, binary size. |
| `accessibility` | Assistive-technology support and audits. |
| `i18n` | Internationalization, localization, RTL, locale handling. |
| `security` | Sandboxing, permissions, supply-chain and data safety. |

### Status

| Label | Meaning |
| --- | --- |
| `pending test` | Implementation landed; the issue stays open until test coverage proves it. |
| `work in progress` | Someone is actively working on it. |
| `good first issue` | Scoped and self-contained enough for a first contribution. |
| `help wanted` | Maintainers would welcome an external contribution. |

### Resolution

Applied when closing without a fix.

| Label | Meaning |
| --- | --- |
| `duplicate` | Already tracked elsewhere — link the canonical issue. |
| `invalid` | Not reproducible, or not a WaterUI problem. |
| `wontfix` | A real finding the project chooses not to act on — say why. |

### Automation (pull requests only — never apply to issues)

| Label | Meaning |
| --- | --- |
| `release` | Marks a `dev` → `main` PR as the intentional version boundary; enforced by `version-boundary.yml`. |
| `nightly-fix` | Exempts a PR from the nightly healthy gate so a fix for a red nightly can merge. |

## Pull requests

One PR resolves one issue. Branch off `dev`, open the PR back to `dev`, and link it with `Fixes #N`. `main` is the release branch — `dev` → `main` merges happen only at a deliberate version boundary.
