# The host interface

`host.js` is the only seam between the JavaScript runtime and the native side.
The Rust host table (water-rs/waterui#1042) is implemented against this
document; the bundled engine calls `installHost(host)` once before evaluating
the `waterui` virtual module, and `uninstallHost()` when the bundle unloads.

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
  - `write(v, x)` — writes a signal or a `{ write }` host value; throws on
    read-only inputs.
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
 * @property {ReadonlySet<string>} modifiers
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

Called once per `mount()`. Returns `{ theme, locale, safeArea }`; each entry
is a constant, a `Signal`, a thunk, or a host-side
`{ read(): T, subscribe(callback): dispose }` value. The runtime materializes
each into a `Signal` seeded from `read()` and updated through `subscribe`, and
publishes them as the built-in contexts behind `useTheme()`, `useLocale()`,
and `useSafeArea()`.

## `modifiers: ReadonlySet<string>`

The set of attribute names that are view modifiers (`"padding"`,
`"background"`, …) — the names the Rust component catalog declares as
modifiers. The catalog is the single source of truth, so the runtime keeps
no table of its own: `installHost` requires this entry, `jsx` uses it to
split modifier attributes from configuration attributes and to drive the
spread backstop, and `Box` uses it to validate its attributes.

## `mount(render) -> { handle, dispose }`

Not part of `Host` — the runtime-side entry the engine calls to mount a
module: creates a detached root scope, seeds the environment contexts from
`host.environment()`, runs `render`, and returns the root handle plus a
`dispose` that tears the whole tree down (branch disposals, subscriptions,
cleanups).
