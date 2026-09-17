# TypeScript views

WaterUI views can be authored in TypeScript with JSX and mounted from Rust.
A `.tsx` module is bundled by the `water` CLI and runs inside an embedded
JavaScript engine; every element it creates is a real `WaterUI` view. There
is no virtual tree and no diffing pass — an element evaluates once into a
native view handle, and reactivity stays fine-grained on both sides of the
bridge: a signal write in TypeScript updates exactly the view that reads it,
and a `Binding` the Rust side pushes reaches TypeScript as a signal.

The vocabulary is the catalog — the component and modifier names encoded in
`waterui::ts::CATALOG` — so the names this guide uses are the ones the
generated `.d.ts` declares. A tag the catalog does not carry is a typed
error, not a fallback.

```tsx
import { Button, Text, VStack } from "waterui";
import type { Signal } from "waterui";

export interface PromoProps {
  headline: string;
  unread: Signal<number>;
  onDismiss: () => void;
}

export default function Promo({ headline, unread, onDismiss }: PromoProps) {
  return (
    <VStack spacing={8}>
      <Text>{headline}</Text>
      <Text>{() => `${unread()} unread`}</Text>
      <Button onTap={onDismiss}>Dismiss</Button>
    </VStack>
  );
}
```

## Attribute order is semantic

Modifier attributes apply in written order, left to right, exactly like a
Rust modifier chain. The two elements below carry the same attributes and
mean two different views — `padding` then `background` insets the text
*inside* the colour, `background` then `padding` hugs the text and leaves
the inset transparent:

```tsx
// text("Attribute order").padding_with(16.0).background(color)
<Text padding={16} background={accent}>Attribute order</Text>

// text("Attribute order").background(color).padding_with(16.0)
<Text background={accent} padding={16}>Attribute order</Text>
```

| `<Text padding={16} background={accent}>` | `<Text background={accent} padding={16}>` |
|---|---|
| ![the colour covers the inset](https://assets.waterui.dev/docs/jsx/jsx-order-padding-then-background.png) | ![the colour hugs the text](https://assets.waterui.dev/docs/jsx/jsx-order-background-then-padding.png) |

Both frames are real renders of the two Rust chains above through the
offscreen backend, produced by the `export_jsx_order_illustrations` test in
`components/platform/ts/tests/illustrations.rs`.

The transform preserves written order by emitting properties in source
order, and the runtime applies them to the element in enumeration order —
this is observable host behaviour, not a style convention. The same rule is
stated in every modifier's generated `.d.ts` doc comment, because that is
where a wrongly ordered pair would be written.

Because order is semantic, a spread may not carry a modifier attribute —
`{...rest}` would hide where `padding` sits relative to `background`. The
transform rejects it statically and the runtime throws as a backstop,
naming the attribute and the element. Configuration attributes
(`spacing={8}`, `onTap={…}`) are not ordered: they configure the component
itself and may come from a spread.

## Signals

Reactivity is the same model as Rust: `Signal<T>` is the `Binding<T>`
shape, `Accessor<T>` is the `Computed<T>` shape.

```tsx
import { createMemo, createSignal } from "waterui";

const count = createSignal(0);
count();              // read — tracked
count.set(1);         // write
count.update(v => v + 1);

const doubled = createMemo(() => count() * 2);   // derived, read-only
const [read, write] = createSignal(10);          // iterating works too
```

Props are getter-props. The transform emits `get padding() { return count() }`,
so a component that reads `props.padding` reads it through an accessor and
the host sees the live value — never a snapshot taken at setup. Writing
`padding={count()}` in the JSX source *is* the live form: the getter is
what the transform emits for it. Anywhere a value is dynamic, the position
accepts `T | Signal<T> | (() => T)` — a constant, a signal, or an accessor.

State that flows back — a toggle's value, a slider's position, a text
field's contents — takes a `Signal`, which is writable:

```tsx
const done = createSignal(false);

<Toggle value={done}>Mark complete</Toggle>
// done() is the value the user left; done.set(true) moves the switch
```

On the Rust side that same prop is a `Binding<bool>`: writes in either
language land in the same cell. Read-only framework values — the theme,
the locale — arrive as `Accessor`s.

## Control flow

Four control-flow components ship with the library. Each branch callback
runs under a fresh reactive scope, so a disposed branch's signals and
`onCleanup`s die with it.

`<Show>` presents its children while `when` is truthy, `fallback`
otherwise. A function child is a render prop that receives an accessor of
the current truthy value:

```tsx
<Show when={loggedIn} fallback={<Text>Sign in</Text>}>
  <Text>Welcome back</Text>
</Show>

<Show when={selected}>{(item) => <Text>{() => item().name}</Text>}</Show>
```

`<For>` is keyed reconciliation over a collection. `each` is the list,
`by` picks the key (default: the item's own identity), and the child is a
render function receiving the item and an accessor of its index:

```tsx
<For each={items} by={(item) => item.id}>
  {(item, index) => <Text>{item.name}</Text>}
</For>
```

Keys must be unique and stable — a `by` that answers the same key for two
rows, or a key outside the domain the host can hold, is an error at mount,
not a silent mis-render.

`<Suspense fallback={…}>…</Suspense>` is the host-driven pending state:
the fallback shows while the children are pending.

## `<Box>`: a modifier scope

`<Box>` creates no node. It takes exactly one host element child and
applies its modifier attributes to that child in written order — the
neutral container for putting modifiers on a subtree or for spelling one
modifier twice, which JSX's duplicate-attribute rule otherwise forbids:

```tsx
<Box padding={8}>
  <Box padding={16} background={accent}>
    <Text>Framed</Text>
  </Box>
</Box>
```

`<Box>` takes only modifier attributes — a configuration attribute or a
non-element child is an error.

## Mounting from Rust

A module is mounted where it is used, with `tsx!`:

```rust
use waterui::tsx;
use waterui::ts::schema::TsProps;
use waterui::Binding;

#[derive(TsProps)]
struct PromoProps {
    headline: String,
    unread: Binding<u32>,
    #[ts(rename = "onDismiss")]
    on_dismiss: Box<dyn Fn()>,
}

let view = tsx!(
    "fixtures/promo.tsx",
    PromoProps {
        headline: String::from("Welcome back"),
        unread,
        on_dismiss: Box::new(move || dismissed.set(true)),
    }
);
```

The path is relative to the Rust file that mounts the module. The module
id the bundle publishes it under is that file's path relative to the
crate's `CARGO_MANIFEST_DIR`, with forward slashes and the extension kept —
`tsx!("fixtures/promo.tsx", …)` written in `tests/mount.rs` mounts
`"tests/fixtures/promo.tsx"`. The macro stats the file to check it exists
and never parses it; the CLI's bundler is what resolves the module graph.

The props argument is a struct literal so the props type is named at the
mount site. `#[derive(TsProps)]` makes it a contract: the TypeScript
interface and the Rust struct are checked against each other by hash —
mounting a bundle built against a different contract is a typed error
naming both hashes. `String` projects to `string`, `Binding<T>` to
`Signal<T>`, `Box<dyn Fn()>` to `() => void`; `#[ts(rename = "…")]` maps a
snake_case field to its camelCase prop. A module that takes no props is
mounted with `tsx!("./about.tsx")` and typed against `NoProps` — the empty
contract, checked like any other.

The expansion also records the mount in a `waterui_meta_tsx_*` static, so
`water build` learns which modules the binary mounts and bundles exactly
those.

### Testing and previewing a mounted module

A `tsx!` view is tested like any other, with `#[waterui::test]` over the
mounted tree, and previewed with `#[preview]`. Both hosts need the bundle:
`water test` builds it and hands its path to the test host in the
`WATERUI_TS_BUNDLE` environment variable, and `water preview` does the same
for the preview host. Under bare `cargo nextest run` nothing sets the
variable, so mounting a module fails with a message naming it and
`water test`; a test that mounts no TypeScript never needs it.

## Contexts

`createContext` / `useContext` are the ordinary context pair, resolved
through the owner chain:

```tsx
import { createContext, useContext } from "waterui";

const Density = createContext("comfortable");

function Row() {
  const density = useContext(Density);
  return <Text>{density}</Text>;
}
```

Two framework contexts are seeded at mount from the host's environment and
are read-only on the TypeScript side:

```tsx
import { useLocale, useTheme } from "waterui";

const theme = useTheme();   // Accessor<{ colorScheme: "light" | "dark", …tokens }>
const locale = useLocale(); // Accessor<{ identifier, languageCode, textDirection }>

<Box background={theme().surface}>
  <Text>{() => locale().identifier}</Text>
</Box>
```

`useTheme()` answers the color scheme and every colour token the
environment installs (`surface`, `accent`, `foreground`, …); a token no
theme installs is absent rather than defaulted. Both throw outside a
mounted tree.

## Localization

`<Text>` localizes through the same `TranslationCatalog` lookup as Rust's
`text("…")`, so one catalog serves both languages of a codebase:

```tsx
<Text>Settings</Text>          // looked up in the catalog for the current locale
<Text>{count()}</Text>         // a number is the value itself, never a key
```

A string child is a translation key, and a key the catalog does not carry
renders as the key itself — exactly what `Text::localized` does. A number
child is verbatim: `<Text>{count()}</Text>` is `text!("{count}")`, a
formatted value rather than a lookup.

## Engines and platform support

The JavaScript engine is selected by target:

- **iOS, macOS, tvOS, visionOS** — the system `JavaScriptCore`, adding no
  binary size.
- **Android, Linux, Windows** — `QuickJS-NG`, embedded.
- **watchOS** — unsupported. The platform ships no `JavaScriptCore`, and
  embedding `QuickJS-NG` is not supported there either, so `tsx!` does not
  compile on watchOS. The asymmetry is documented, not faked.
