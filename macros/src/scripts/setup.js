// The script the `js_file!` / `exec_file!` doc examples read at compile time.
// It exists so those examples are compiled rather than ignored: both macros
// expand to `include_str!`, which needs a real file next to the caller.
globalThis.app = globalThis.app ?? {};
