# WaterUI Logging & Panic Reporting Plan

## Current State

### FFI (`ffi/src/lib.rs`)

- `install_panic_hook()` is called in `__init()`
- Sets up `tracing_panic::panic_hook` to convert panics to tracing events
- Recently added `tracing_subscriber` initialization (but this conflicts with hot_reload)

### Hot Reload (`src/hot_reload.rs`)

- `install_panic_forwarder()` - Installs custom panic hook, sends structured `PanicReport` to CLI
- `install_tracing_forwarder()` - Sets up complete tracing pipeline:
  - `console_layer` - Outputs to stderr (with `[waterui::tracing]` prefix)
  - `cli_layer` - Forwards to CLI via websocket
  - `PanicAwareFormatter` - Special formatting for panic events (simplified output)
- `ClientEvent::Panic` - Structured panic info (file, line, column, backtrace)
- `ClientEvent::Log` - Structured log events

## Problems

1. **Duplicate subscriber initialization conflict**
   - FFI `__init()` initializes a simple subscriber
   - Hot reload `install_tracing_forwarder()` tries to initialize again, fails
   - Result: In hot reload mode, CLI cannot receive logs

2. **No logs in non-hot-reload mode**
   - Without hot reload, only FFI's simple subscriber exists
   - Panic info outputs to stderr, but poorly formatted
   - Cannot send to CLI

3. **Log duplication when CLI is connected**
   - System layer outputs full logs to stderr/logcat
   - CLI also echoes app logs to terminal
   - Results in duplicate log display

## Proposed Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                          App Process                            │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Tracing Pipeline                      │   │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐   │   │
│  │  │ System Layer │  │  CLI Layer   │  │ Panic Hook   │   │   │
│  │  │(stderr/logcat)│  │ (websocket) │  │ (structured) │   │   │
│  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘   │   │
│  │         │                 │                  │           │   │
│  │         ▼                 ▼                  ▼           │   │
│  │   Platform Logs     WebSocket Channel   Panic Report    │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                  │
└──────────────────────────────┼──────────────────────────────────┘
                               │
                     ┌─────────▼─────────┐
                     │   CLI Process     │
                     │  ┌─────────────┐  │
                     │  │ Log Viewer  │  │
                     │  │  (pretty)   │  │
                     │  └─────────────┘  │
                     │  ┌─────────────┐  │
                     │  │Panic Report │  │
                     │  │ (highlight) │  │
                     │  └─────────────┘  │
                     └───────────────────┘
```

## Implementation Plan

### Phase 1: Fix FFI/Hot-Reload Conflict

**Goal**: Ensure both hot reload and non-hot-reload modes have proper logging

1. **FFI `__init()` should not initialize subscriber**
   - Remove `tracing_subscriber` initialization from FFI
   - Keep only `tracing_panic::panic_hook`

2. **Lazy subscriber initialization**
   - Initialize subscriber on first tracing event
   - If hot reload is available, use the full pipeline
   - Otherwise, use simple stderr output

### Phase 2: CLI-side Log Filtering

**Goal**: Show pretty websocket logs for Rust tracing, keep system messages, avoid duplicates

Key decisions:
1. **`water run` always enables hot reload** - remove `--no-hot-reload` flag
2. **Websocket disconnect = CLI exits** - if retry fails, exit (no fallback mode)
3. **System logs always full** - output everything with `[waterui::tracing]` prefix
4. **CLI always filters** - filter `[waterui::tracing]` lines from system logs

This is simple because:
- CLI always has websocket connection (or it exits)
- CLI always gets Rust logs via websocket (structured, pretty)
- CLI always filters raw tracing from system logs (no duplicates)

Implementation:

1. **Rust side (no changes needed)**
   - System layer always outputs to stderr with `[waterui::tracing]` prefix
   - CLI layer forwards structured logs via websocket
   - Both always active

2. **CLI side**
   - Remove `--no-hot-reload` flag from `water run`
   - When reading system logs, filter out lines containing `[waterui::tracing]`
   - Display websocket logs (pretty, structured)
   - On websocket disconnect (after retry fails): exit CLI

3. **Result**
   - System messages: displayed (iOS/Android framework logs, crash reports)
   - Rust tracing: displayed via websocket (pretty format)
   - No duplicates, no conditional logic

### Phase 3: CLI Panic Report Enhancement

**Goal**: Display beautiful crash reports when CLI receives a panic

1. **Parse `PanicReport`**
   - Extract file, line, column
   - Parse backtrace to get all stack frames

2. **Source Code Highlighting**
   - Read source file
   - Highlight the error line
   - Show context code around it

3. **Interactive Features**
   - Click stack frame to open editor
   - Fold/unfold backtrace
   - Copy error message

## File Changes

### `ffi/src/lib.rs`

- Remove `tracing_subscriber` initialization
- Keep `tracing_panic::panic_hook`

### `src/hot_reload.rs`

- Add `is_cli_connected()` function
- Keep full output in `PanicAwareFormatter` regardless of connection status
- Add fallback subscriber initialization

### `cli/src/terminal/command/run.rs` (or related files)

- Add `--echo-logs` / `--no-echo-logs` flag
- Add panic report parsing and display logic
- Add source file reading and highlighting

## Priority

1. **High**: Fix current panic not showing issue (Phase 1)
2. **Medium**: Add log echo control flag to CLI (Phase 2)
3. **Low**: Pretty crash reports in CLI (Phase 3)
