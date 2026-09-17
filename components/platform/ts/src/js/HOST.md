# The host interface

`host.js` is the only seam between the JavaScript runtime and the native side.
The Rust host table (water-rs/waterui#1042) is implemented against this
document. The bridge evaluates the bundle, reads
`globalThis.__waterui_runtime` (below), and calls `installHost(host)` once
through it — before any module is mounted, which is the first moment anything
asks for the host — and `uninstallHost()` when the bundle unloads.

Everything the runtime hands the host is one of four shapes:

- **`Handle`** — an opaque native view handle produced by the host. The JS
  side stores and passes it but never inspects it. Handles are objects;
  `<Box>` relies on that to reject primitive children.
- **`Branch`** — `{ handle: Handle, dispose(): void }`. Each control-flow
  render callback runs under a fresh reactive scope and returns a `Branch`.
  The host owns *when* a branch is presented and calls `dispose` exactly once
  when the branch stops being presented; `dispose` runs the branch's
  `onCleanup`s and tears down its signals and effects.
- **Reactive input** — anywhere a value is dynamic the runtime passes the
  user-supplied form through verbatim:
  `T | Signal<T> | (() => T) | { read(): T, subscribe?(cb): dispose }`. The
  host distinguishes and materializes them with the helpers exported from
  `host.js`:
  - `isSignal(v)` / `isAccessor(v)` — classify. A `Signal` is callable
    (`v()`) and writable (`v.set`, `v.update`); a plain function or a
    `{ read() }` object is a read-only accessor. Anything else is a constant.
  - `read(v)` — current value, untracked; constants pass through.
  - `write(v, x) -> boolean` — writes a signal or a `{ write }` host value;
    throws on read-only inputs. `x` is stored verbatim: a signal's `set` reads
    a function argument as an updater, so `write` passes one that returns `x`,
    and a callback or a memo crossing from Rust is stored rather than called.
    The answer is whether the value stood: `true` when reading `v` back gives
    exactly `x`, `false` when an effect changed it while the write settled.
    Equality is the target's own comparator — SameValue unless the signal was
    created with another — so the two sides never disagree about what counts
    as a change.
  - `subscribe(v, callback) -> dispose` — runs `callback(value)` on every
    settled change, glitch-free, with no initial call. This is the push half
    of the `Signal<T> ↔ Binding<T>` mapping: the bridge materializes a JS
    accessor as a Rust `Computed<T>` by subscribing and writing into the Rust
    side, and materializes a `Signal<T>` as a `Binding<T>` by additionally
    forwarding Rust writes through `write`.
  - `toAccessor(v)` — lifts any reactive input to a plain `() => T`.
  - `toSignal(v)` — materializes any reactive input as a reactive value the
    runtime tracks: signals and memos pass through, `{ read, subscribe }`
    values become push-fed signals (the subscription dies with the current
    owner), thunks become memos, constants become constant signals.
    `mount`'s environment seeding and the reference host in the test suite
    both go through it.
- **Plain values** — configuration data crossing as ordinary JS values.

```js
/**
 * @typedef {object} Host
 * @property {(component: string, config: object, children: unknown[]) => Handle} create
 * @property {(handle: Handle, name: string, value: unknown) => Handle} modify
 * @property {(content: unknown) => Handle} text
 * @property {(when: unknown, render: (item: () => unknown) => Branch, fallback?: () => Branch) => Handle} show
 * @property {(each: unknown, render: (item: unknown, index: () => number) => Branch, by?: (item: unknown) => unknown) => Handle} each
 * @property {(children: () => Branch, fallback?: () => Branch) => Handle} suspense
 * @property {() => HostEnvironment} environment
 * @property {(id: number, ...args: unknown[]) => unknown} invoke
 * @property {ReadonlySet<string> | readonly string[]} modifiers
 */
```

## `create(component, config, children) -> Handle`

Creates a native view. `component` is the WaterUI component name from the JSX
tag (`"VStack"`, `"Text"`, `"Toggle"`, …) — the host resolves it against the
component catalog and must throw on an unknown name.

`config` carries the component's configuration attributes. The transform's
getter-props are already resolved: a dynamic attribute arrives as an
accessor, so every config value is a reactive input
(`T | Signal<T> | (() => T)`) or a plain constant — the host never sees a
property getter and must not read the property eagerly. `on*` attributes are
event callbacks and are invoked, never subscribed.

`children` is the normalized child list: an array whose elements are
`Handle | string | number | boolean | null | Accessor<…>`. `null`, `undefined`,
and booleans are already filtered; nested arrays are already flattened. String
and number elements are materialized with `text`. An accessor element is a
reactive child slot: the host subscribes to it and swaps the child when it
produces a new element (an accessor may also yield a list, which the host
normalizes the same way). For components with a label slot (`Button`,
`Toggle`, …) the children are the label; `config.label` is the explicit form
and takes precedence when both are present.

## `modify(handle, name, value) -> Handle`

Applies modifier `name` (`"padding"`, `"background"`, `"foreground"`, …) to
the view and returns the resulting handle — which may be the same handle or a
new one; callers always use the return value.

Modifier attributes apply in written attribute order, left to right, exactly
like a Rust modifier chain. The runtime guarantees the calls arrive in that
order; the host applies them in receive order. `value` is a reactive input.

A modifier attribute that arrives through a JSX spread is rejected by the
runtime before any host call — the host never has to detect that case.

## `text(content) -> Handle`

Materializes a text leaf. `content` is `string | number | Accessor<string>`
and participates in the same localization lookup as Rust `text(…)`/`text!`.
A reactive `content` updates the leaf in place.

## `show(when, render, fallback?) -> Handle`

Maps to WaterUI's `When`. `when` is a reactive input; while it reads truthy
the host presents `render(item)` where `item` is an accessor yielding the
current value, otherwise it presents `fallback?.()` or nothing. Each
activation calls the callback once and presents the returned branch's handle;
each deactivation calls the branch's `dispose`. Re-evaluating `when` while it
stays on the same side does not recreate the branch — the value accessor
carries the update.

## `each(each, render, by?) -> Handle`

Maps to WaterUI's `AnyViews` identity reconciliation. `each` is a reactive
input reading the item list. Identity is `by(item)` when given, referential
identity otherwise. On every change the host reconciles: a retained key keeps
its branch (the branch's `index` accessor updates if its position moved), a
new key calls `render(item, index)` once, and a removed key's branch is
disposed. `render` receives the item and an accessor of its current index.

## `suspense(children, fallback?) -> Handle`

Maps to WaterUI's `Suspense`. The host presents `fallback?.()` while the
`children()` branch's resources are pending and the children branch once
resolved, disposing whichever branch leaves.

## `environment() -> HostEnvironment`

Called once per `mount()`. Returns `{ theme, locale }`; each entry is a
constant, a `Signal`, a thunk, or a host-side
`{ read(): T, subscribe(callback): dispose }` value. The runtime materializes
each into a `Signal` seeded from `read()` and updated through `subscribe`, and
publishes them as the built-in contexts behind `useTheme()` and `useLocale()`.

`theme` carries `{ colorScheme: "light" | "dark" }` plus the theme's color
tokens keyed by slot name (`foreground`, `background`, `surface`,
`surfaceVariant`, `border`, `accent`, `mutedForeground`, `accentForeground`,
`accentContainer`, `tertiary`, `tertiaryContainer`, `selectionContainer`,
`selectionForeground`, `error`, `errorForeground`), each
`{ red, green, blue, headroom, opacity }` in linear light. A slot the
environment does not install is absent rather than defaulted. `locale` carries
`{ identifier, languageCode, textDirection }`.

There is no `safeArea` entry, and no `useSafeArea()`. WaterUI publishes no
ambient inset value: a backend places content clear of the hardware at the
container level — a stack lays its children out inside the safe area and
extends the scroll surfaces and chrome containers that touch its edges — so
neither the framework nor a view reads an inset number. An accessor that could
only ever answer zeroes would fake a primitive that does not exist, so the
asymmetry is documented rather than hidden.

## `invoke(id, …args)`

Dispatches one Rust closure from the bridge's registry. It is the bridge's own
entry rather than the catalog's, and it is in the table for the reason given
under the runtime global: what the runtime calls must be what the engine
registered.

## `modifiers: ReadonlySet<string> | readonly string[]`

The attribute names that are view modifiers (`"padding"`, `"background"`,
…) — the names the Rust component catalog declares as modifiers. The catalog
is the single source of truth, so the runtime keeps no table of its own:
`installHost` requires this entry, `jsx` uses it to split modifier attributes
from configuration attributes and to drive the spread backstop, and `Box` uses
it to validate its attributes.

A `Set` is not a plain object and therefore cannot cross the engine seam as a
value, so the Rust host table sends the names as an array and `installHost`
builds the set once, at install time — classification stays a JS-local lookup
instead of a boundary crossing per attribute. Because the runtime keeps the
table with that entry normalized, and may hold a copy of the object to do so,
every host entry must be a plain function that does not depend on `this`.

## `globalThis.__waterui_runtime`

A bundle is a classic script, so nothing it declares is reachable from Rust.
The bundle entry the CLI generates therefore ends with one call —
`installRuntimeGlobal(modules)` — which publishes the runtime on
`globalThis.__waterui_runtime`. The bridge reads the global right after
`eval` and refuses a bundle that is missing an entry, naming it.

| Entry | What the bridge does with it |
| --- | --- |
| `installHost(host)` / `uninstallHost()` | Installs the table above; the bridge installs once, after `eval`. |
| `mount(render)` | Mounts one module under a fresh root scope. |
| `isSignal(v)` / `isAccessor(v)` | Classifies a reactive input that crossed as a function handle. |
| `read(v)` | Seeds a materialized cell, untracked. |
| `write(target, value)` | Pushes a Rust value into a JS signal, and answers whether it stood. |
| `subscribe(source, callback)` | The push half of the mapping; returns the dispose the cell owns. |
| `toSignal(v)` / `toAccessor(v)` | Used by the runtime itself; the bridge only requires them to be present. |
| `createSignal(value)` | Creates the JS signal a Rust `Binding<T>` is exported as. |
| `createMemo(compute)` | Wraps the pushed signal a Rust `Computed<T>` is exported as, so JS sees a read-only accessor. |
| `makeCallback(id)` | The JS function wrapping a Rust closure held in the bridge's registry. |
| `modules` | Module id → that module's default export. |

`makeCallback(id)` calls the installed host's `invoke(id, …args)`, the one
entry the bridge registers for every Rust closure JavaScript calls, from a prop
callback to a signal subscription. It is taken from the table, not from
`globalThis.__waterui_host`: that global is writable, and a bundle that
reassigned an entry of it while it evaluated would otherwise become what every
callback wrapper — and every host call — dispatches through. The bridge
captures each host function as it registers it, before any bundle exists, and
installs those.

## Materialization, and what keeps it from oscillating

A reactive input becomes a Rust signal only when it reaches a native view —
inside a host call, never while props are converted — so an input the tree
never uses creates nothing on the Rust side.

- A `Signal` becomes a `Binding<T>`: the bridge subscribes with a registry
  callback, converts each pushed value and `set`s the binding; a nami `watch`
  on the binding sends Rust-side changes back through `write`.
- An accessor — a memo, a thunk, or a `{ read, subscribe }` value — becomes a
  `Computed<T>` fed the same way, with no write-back.
- Anything else is a constant and becomes a constant `Computed<T>`.

Backends read the Rust value: a `get` never crosses into the engine.

Both directions are guarded so one change propagates once. While an inbound
value is being applied the write-back is suppressed, which is what stops a
`Binding::set` — nami notifies unconditionally, `distinct` is opt-in — from
bouncing straight back. While an outbound value is being written the
subscription is suppressed outright, for the whole write: a JS write settles
its effects synchronously, and how many notifications that takes, and in what
order, is JavaScript's business. What the bridge acts on is where the value
came to rest, which is what `write` answers. When the value stood there is
nothing more to do; when an effect changed it, the bridge applies the settled
value under the inbound guard and pushes what the binding holds afterwards,
which is the same step again. The loop is bounded: two sides that answer
every value with a different one are a cycle, reported with the binding's
identity rather than ridden into a stack overflow.

The comparison is made here rather than in Rust because it turns on object
identity: a signal, a view slot or a callback crossing back out to Rust is a
fresh handle there, and only JavaScript can see that two references are the
same object. Classifying each notification against a remembered value instead
would also mistake a real correction for an echo whenever the pushed value
reappears later in the same settle.

The equality both sides settle on is SameValue, which is why this runtime's
signals default to `Object.is` rather than the `===` most signal libraries
use. It differs on exactly two values, and both matter here: `NaN` equals
itself, so a float written twice propagates once instead of on every write,
and `-0` is not `0`, so a Rust value written over its opposite sign is a real
change rather than a write JavaScript drops and Rust is then corrected out of.

That is the equality a signal uses to decide whether a set is a change. The
skip in `write` is a different question — "is this the value already here?"
— asked of a value that has just crossed the seam, so it uses `bridgeEquals`:
primitives, arrays and plain objects compare structurally, because a payload
crossing from Rust is a fresh copy every time, and functions, handles and
class instances compare by identity, because those cross as handles and a
copy would be a different object. The comparison is depth-bounded; a cyclic
graph cannot cross the seam in the first place.

A cell also always pushes what its binding holds at the moment its watcher
runs, never the value the notification carried: a watcher registered earlier
may have written again, and a late notification must not resurrect the value
it was raised with. Suppression during an inbound apply is a blanket: the
watch pushes nothing while the value is going in, and the cell pushes what
the binding holds once the apply is over. That covers every way the binding
can hold something else than what JavaScript sent — a `filter` that rejected
the write, a mapping binding whose setter normalized it, another watcher that
answered it — without counting notifications, which a setter is free to raise
any number of times, including none. The push costs nothing when the value
did stand, because `write` skips a write of a value the target already holds.

Every materialized cell owns its subscription's dispose function and its watch
guard, and disposes the JavaScript subscription when it is dropped.

The values travelling the other way — a `Binding<T>` exported as a JS signal,
a `Computed<T>` as a memo, a Rust closure as a callback — are owned by nobody
on the Rust side, because JavaScript holds them. They belong to the scope of
the mount they were exported for, and disposing that mount releases every one
of them; exporting with no mount open is an error rather than a leak that
lives as long as the runtime. Inside one mount a Rust signal exported twice is
the same JS signal both times, so a value pushed on every change does not
leave a trail of signals and cells behind it.

## `mount(render) -> { handle, dispose }`

Not part of `Host` — the runtime-side entry the engine calls to mount a
module: creates a detached root scope, seeds the environment contexts from
`host.environment()`, runs `render`, and returns the root handle plus a
`dispose` that tears the whole tree down (branch disposals, subscriptions,
cleanups).
