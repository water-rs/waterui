# waterui-qr

QR code rendering for WaterUI, drawn as a scene rather than as an image.

```rust
use waterui_qr::{ErrorCorrection, qr_code};

qr_code("https://waterui.dev").correction(ErrorCorrection::High)
```

## How it works

```
payload ──(qrcode)──► QrMatrix ──► snapped grid ──► Scene2D rects
   │                                                     │
   └──► a11y label                                       └──► any backend
```

No platform ships a QR primitive — a code is a picture everywhere — so this is a
self-drawn component, exactly as `AGENTS.md` prescribes for a semantic component
with no native counterpart. Encoding is `qrcode`'s: the bit stream, the
error-correction blocks, the mask selection and the placement rules of ISO/IEC
18004 are a specification with a maintained implementation, and only its module
grid is taken. The grid is drawn through the engine-independent `Scene2D`
contract, so a code renders on the classic compute pipeline, on the CPU/GPU
split engine that adapters without compute shaders fall to, and on dew's CPU
scene, and stays vector sharp at any size instead of being rasterized once.

## Snapping

One module is a whole number of units on a side, and the block is placed at a
whole-unit origin, so every module boundary is exact. This is what keeps a code
scannable rather than merely tidy: a boundary that falls inside a pixel is
antialiased into a grey seam, and a decoder's binarizer then has to guess which
side that seam belongs to. At small sizes it guesses wrong on most rows, which
is more damage than the error correction was sized for.

The space being snapped to is the one the scene is built in — the surface's own
pixel grid where the scene owns a `GpuSurface`, and the window's logical grid
(the device pixel grid at every integer scale factor) where a self-drawn backend
merges the scene into its own. Whatever the box does not divide evenly is left
around the symbol as extra clear space, which is the one thing around a code
that may grow freely.

## The quiet zone

Four modules on every side, per the specification, drawn as part of the code
rather than left to the caller: a required margin that has to be remembered is a
margin that gets forgotten, and a code pressed against neighbouring content is
one a detector finds late or not at all. `quiet_zone(n)` changes it for a code
that sits on a ground that is already clear.

## Colours

A QR code is a machine-readable target, so its two colours are functional rather
than decorative. The defaults come from the theme — `Foreground` and `Surface`,
so a themed application gets its own near-black and near-white rather than a
hardcoded `#000` on `#fff` — but the *darker* of the two always draws the
modules.

That ordering is the whole point. A decoder looks for dark modules on a light
ground, and the detectors in wide use (`quirc`, ZXing, and the `rqrr` this
crate's tests decode with) do not search for the reflectance-reversed form at
all, so `Foreground` on `Surface` taken verbatim would hand a dark-mode
application a code that looks right and does not scan. `tests/round_trip.rs`
pins both halves: a code under the dark theme decodes, and a deliberately
inverted one is not found.

`module_color(…)` and `background_color(…)` take the judgement back; both accept
signals, so a code follows a colour that changes without its subtree being
rebuilt.

## Reactivity

The payload and both colours are signals. A change to any of them invalidates
the scene rather than rebuilding the subtree, so the content instance and its
cached grid survive it and only the frame is redrawn.

## Accessibility

A code reaches the screen as an anonymous field of squares, and unlike a
decorative drawing it *is* content: the whole point of the picture is a string
that only a camera can get at. The drawing answers
`SceneContent::accessibility_label` with the payload and the backend names the
node with it, so anyone who cannot point a camera at the screen still gets what
the code says. `.a11y_label("Boarding pass")` wins wherever the application
names the code itself — the payload is what the node says when nobody did.

## Failure

A payload past the largest symbol at the chosen level is a `QrError`. There is
no truncation and no drop to a weaker correction level: a code that says less
than the payload is a code that sends whoever scans it somewhere else.

## Tests

`tests/round_trip.rs` renders the component offscreen and decodes the pixels
back with `rqrr`, at two module sizes and under two themes. It is the only
assertion that says what matters — every way a drawn code stops being readable
leaves a picture that still looks like a QR code — and it writes the frames into
WaterUI's canonical artifact layout, so the same run that proves a code decodes
also leaves the image to look at.
