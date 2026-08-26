# skill_snippets — the WaterUI skill's compile gate

Every fenced ` ```rust ` block in `.claude/skills/waterui/` is transcribed into this
crate and compiled by CI. If a skill snippet stops compiling, this crate stops
compiling, and the pipeline says so before a reader copies broken code out of the
documentation.

The gate is not hypothetical: across two audit rounds it caught **31 real defects** in
the skill — wrong argument types and arities, methods that did not exist, use-after-move,
missing `move` on a `'static` closure, imports the prose never mentioned, and a
same-named-trait footgun that made `.animated()` mean two different things depending on
which trait was in scope. Three of those turned out to be framework bugs rather than
documentation bugs and were fixed in the framework.

## Running it

```bash
cargo check -p skill_snippets --all-targets
cargo check -p skill_snippets --all-targets --features compile-gate-tests
cargo clippy -p skill_snippets --all-targets -- -D warnings
```

CI runs the second command; it is a superset of the first.

### The `compile-gate-tests` feature, and the never-execute rule

`#[waterui::test]` and `#[waterui::bench]` transcriptions sit behind the **non-default**
`compile-gate-tests` feature so the workspace-wide `cargo nextest run` never compiles or
registers them.

**They must never be executed.** The query, interaction and waiting listings are
catalogues of method names, and the accessibility elements they address do not exist in
any particular view — running them would panic for reasons that say nothing about
whether the documented API exists. The gate is a *compile* gate: `cargo check` with the
feature, never `cargo nextest run` with it.

## Layout

One module per skill file, blocks in file order:

| Module | Skill file |
|---|---|
| `src/skill_md.rs` | `SKILL.md` |
| `src/ref_reactivity.rs` | `references/reactivity.md` |
| `src/ref_components.rs` | `references/components.md` |
| `src/ref_media.rs` | `references/media.md` |
| `src/ref_interaction.rs` | `references/interaction.md` |
| `src/ref_navigation.rs` | `references/navigation.md` |
| `src/ref_styling.rs` | `references/styling.md` |
| `src/ref_i18n.rs` | `references/i18n.md` |
| `src/ref_testing.rs` | `references/testing.md` |
| `src/ref_project.rs` | `references/project.md` |
| `src/ref_troubleshooting.rs` | `references/troubleshooting.md` |

Every transcription is preceded by a banner comment naming its source precisely:

```rust
// ---------------------------------------------------------------------------
// components.md § "## Controls" — rust block 12/28
// ---------------------------------------------------------------------------
```

so a change to one skill section maps to one obvious spot here. Passages that are prose
rather than a fenced block, but that still name an API, are transcribed too and labelled
`(prose)` with a note that they are **not counted** among the file's rust blocks.

## Transcription rules

These are the point of the crate. A transcription that has been "fixed up" proves
nothing about the skill.

1. **Snippets are copied verbatim.** The only permitted normalization is `rustfmt` (see
   below).
2. **Glue goes *around* a snippet, never inside it.** A wrapping `fn`, `let` bindings for
   the free variables the snippet references, and the `use` items the skill's own prose
   says are in scope. Glue picks the types the surrounding prose implies.
3. **Ellipsis markers are the only substitution permitted inside a snippet** — `…`,
   `/* … */`, `...`, `<crate-version>`-style placeholders. Each carries a trailing
   `// [ellipsis filled]` so the fills are auditable by grep.
4. **A snippet that cannot compile as written is a finding, not a bug to patch.** Comment
   it out under a `// SKILL-BUG:` marker with the exact compiler error, report it, and let
   the skill be fixed. Never edit the snippet into compiling. (There are currently no
   such markers: every one raised so far has been fixed at the source.)
5. **Listing blocks are split one item per expression.** Several fences are API-shape
   catalogues rather than statement sequences — consecutive lines with no semicolons, a
   receiver reused after being moved, or one line holding `/`-separated alternatives:

   ```
   .size(w, h) / .width(w) / .height(h) / .min_width(w) / .max_width(w)
   ```

   Each alternative becomes its own expression against a fresh receiver, because that is
   what proves each method individually. Bare method fragments get a receiver supplied by
   glue; the receiver is chosen to avoid inherent-method shadowing (`Divider`, not
   `text(..)`, since `Text::size` is the *font* size).
6. **Some blocks are not compilable by design.** A cfg-gated Apple-only import, or a call
   into a backend crate an app does not depend on. Those are recorded in a comment
   explaining why, not transcribed.

### The rustfmt caveat

The crate is `rustfmt`-clean, and rustfmt rewrites transcribed lines: it collapses the
alignment of trailing comments, re-wraps long argument lists, expands one-line struct
bodies, and inlines short `let _ = { … };` wrappers. **That normalization is a sanctioned
deviation** — it changes layout, never tokens.

The accounting for the current skill text, over 874 snippet lines in 135 blocks:

| Class | Lines |
|---|---|
| byte-identical to the skill | 509 |
| identical after whitespace normalization (rustfmt) | 249 |
| restructured: listing splits, ellipsis fills, rustfmt re-wraps | 116 |

When reviewing a change here, compare *tokens*, not columns.

## Assets

Some snippets resolve real paths, so the crate carries the files they name:

- `i18n/en.toml`, `i18n/de.toml` — the exact `text!` keys i18n.md's snippets use,
  including the CLDR plural table for `"I have {#count} passport stamp"`. The macro
  parses these at compile time, so a malformed table is a build failure.
- `src/guide.md` — for `include_markdown!("guide.md")`.
- `src/starfield.wgsl` — copied from `examples/starfield`, for `shader!("starfield.wgsl")`.

## Dependencies

Beyond `waterui` with every feature the skill documents, the crate depends on exactly the
crates the skill's snippets name by path (`waterui-canvas`, `waterui-graphics`,
`waterui-locale`, `waterui-map`, `waterui-map-gpu`, `waterui-url`, `waterkit-permission`,
`jiff`, the two icon sets). `serde` is glue for media.md's `#[js_api]` payload type.

The crate deliberately does **not** opt into `[lints] workspace = true`: the workspace's
pedantic and nursery lints would demand rewriting verbatim snippet text. It is clean under
default clippy with `-D warnings` instead, with no `allow` attributes — the few genuine
conflicts carry a narrowly scoped item-level `#[expect(..., reason = "…")]` naming the
verbatim constraint.

## Changing a skill snippet

1. Edit the skill file.
2. Regenerate the matching module here, following the rules above.
3. `cargo check -p skill_snippets --all-targets --features compile-gate-tests`
4. `cargo clippy -p skill_snippets --all-targets -- -D warnings`
5. `rustfmt --edition 2024 examples/skill_snippets/src/*.rs`

If step 3 fails, the skill is wrong. Fix the skill, not the transcription.
