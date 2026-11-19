# waterui-meta

Artifact linker-section channels shared across WaterUI crates and tooling:

- **`.wmeta` (`__DATA,__wmeta` / `__DWARF,__wmeta` on Apple)**: `#[used]`
  `name\0payload\0` records emitted by procedural macros (`waterui_meta_*`) for
  the `water` CLI and build tooling.
- **`.wreach` (`__DATA,__wreach` on Apple)**: un-`#[used]` `name\0`
  reachability markers referenced via `core::hint::black_box` at realization
  and erased-dispatch entries, retained in the linked image if and only if the
  referencing code survives dead-code elimination.

Both channels identify records from section bytes rather than the symbol
table, which linked PE images omit and `strip = true` release builds remove.

Zero dependencies, `no_std` + `alloc`.
