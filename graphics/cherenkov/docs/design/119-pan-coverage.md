# #119 pan coverage: bounded atlas eviction

Panning a path-heavy layer used to re-rasterize every path per frame.
Two mechanisms produced that cost. `snap_animating` quantized only the
transforms the engine itself drives, so a host-side `Motion::Transform`
pan kept arbitrary subpixel translations and `path::placement` hashed
each distinct fraction as a different key — the legitimate part of the
rasterization. The illegitimate part: the glyph atlas's only answer to
an overflowing pending batch was `AtlasPlan::Recycle` — a wholesale
`atlas.clear()` that also bumped `generation`. Every retained `Emission`
carries that generation, so the clear failed every emission's liveness
check and forced a full re-lower of the frame, not just a re-rasterize
of the batch that overflowed. On `scenes/perf/map-pan` the atlas cleared
about every third moving frame and re-rasterized ~85% of paths.

## Design

Replace wholesale recycling with bounded in-place eviction at shelf
granularity (plan-of-record candidate C; A and B landed via #186).

- `Layout` already described the atlas as shelf bands; dead bands are
  now a first-class `vacant` set. `Atlas::plan` dry-runs the pending
  batch on a cloned layout — strict allocation first, then simulated
  eviction — and returns `Fits`, `FitsEviction`, `Grow(size)` or
  `Recycle` exactly matching what the commit would do.
- The commit's work scales with what changed, not what is retained.
  `fits_strict` dry-runs the batch alone first; a strictly-fitting
  frame goes straight to `begin_commit(&[])` — no emission scan, no
  pin derivation, no eviction. Only a batch that cannot place strictly
  runs `replay_pins` — every retained emission's glyph instances and
  the frame's masked instances recover their shelves from stored UVs
  (`Atlas::shelf_at`) — and plans with eviction enabled. The scan's
  buffers (`commit_touches`, `commit_writes`, the dedupe sets and plan
  cells) live on the renderer and are rebuilt in place, so the steady
  commit allocates nothing.
- A commit that needs room calls `enable_evicting`, then `alloc` falls
  through live-fit → vacant best-fit → virgin top → `evict_one`. The
  victim is the coldest shelf by segmented LRU: probationary
  (`hits == 0`) before protected, then least-recently-used within a
  segment. A shelf touched by a hit or written this commit
  (`last_used == tick`) can never be the victim, so eviction frees only
  what the frame itself did not ask for.
- `free_band` coalesces dead bands vertically adjacent to the freed
  one — without it eviction leaves space fragmented by height class
  and tall cells starve even while bytes are free — and a merged band
  reaching the layout frontier hands its rows back to `top` as virgin
  space. The vacant set stays pairwise non-adjacent, which bounds the
  merge scan.
- Eviction does not touch `generation`. Surviving emissions stay live.

## Correctness: shelf epochs and per-leaf refs

A baked cell index is valid only while its band still occupies the same
atlas slot, so every emission that drew atlas cells eventually records
`(slot, epoch)` pairs — `Emission.refs`, one packed `u64`
(`start << 32 | len`) addressing the shared `EmissionStorage.refs`
arena — one per band it used. `Shelf.epoch` comes from
`Layout.next_epoch` and is bumped on every band admission (vacant
reuse, top carve, phantom split) and on every band death
(`free_band`), so a slot re-used by a different band always reports a
different epoch. Liveness is one packed compare:
`Emission.live_stamp` mirrors `Atlas::live_stamp()` — texture
`generation` in the high half, eviction `clock` in the low — and hits
check equality first; only an emission whose stamp is stale walks its
refs (`shelf_epoch(slot) == epoch` per ref — no hash lookups),
restamping on success.

Pair collection is deferred out of `lower` entirely — and so is slot
collection. Lowering runs parallel against an immutable `&Atlas`, and
epochs cannot be sampled there anyway, so the leaf records nothing
about which shelves its cells landed on: `refs = DEFERRED_REFS`, a
bare flag. At apply, `apply_pending` derives the pairs two ways: each
rasterized origin's bands through `pending_cells`, and
`Atlas::shelf_at(uv)` over the emission's glyph-kind retained
instances — glyph cells and replayed path quads alike store their
cell's atlas pixel in `uv`, and consecutive quads share a shelf, so a
one-element hint keeps the recovery a compare in the common case. The
fold writes `(slot, epoch)` pairs contiguously at the storage tail,
then repacks `refs` into a plain pair range. `live_stamp` is the
produce frame's stamp, so a not-yet-applied emission can hit inside
its own frame while its commit is in flight; `stale_live` refuses a
deferred emission on any later frame, because unapplied claims are
frame-scoped.

`pending_cells` packs the producing leaf's `(inst_base << 32 |
start << 11 | len)` — verbatim frame indices into the producing
frame's `Lowering::emission_patches` — resolved at apply into
storage-local uv patches and cleared there, never carried past the
commit. `emission_patches` is produce-scoped and append-only: the
emit sites write it beside `cell_patches`, and the frame-scoped
rollbacks (`realize_clipped_leaf`, `try_passthrough`) truncate
`cell_patches` but never this list, so an emission produced inside a
rolled-back scope keeps a valid range — `pending_cells` always
belongs to the lowering that filled the list. A same-frame recompose
re-emits frame patches from it, rebased by `wrapping_sub/add` on the
packed instance base — unconditionally, since a clipped recompose
lands on the same base with its originals already truncated. Every
path that skips apply — a `Grow` early return, an apply abandoned on
`AtlasExhausted`, or a lowering that returned `Err` — discards that
surface's retained emissions (`discard_surface`), so stale pending
cells can never compose into a later frame. Off-surface paths store
nothing at all: an empty emission carries no bands, so an
offset-keyed insert would never hit again and, having no slot, could
never be evicted — it would only grow the `paths` map forever.

Clip masks bind their cell through `uv[2..3]` on each masked frame
instance, so `replay_pins` recovers their shelf the same way — no
slot is recorded at emit time — and the mask's own `(slot, epoch)`
enters the emission's refs fold through the pending-cells origins.

`emit_slots` ranges of evicted emissions are recycled: `evict_one`
pushes the range onto `slot_dead`, `begin_commit` promotes it to
`slot_holes`, and admission reuses a hole before growing the arena —
same-commit readers keep addressing it correctly because a freed range
is never handed out until the next commit begins.

## Residual behavior

`Recycle` remains as the plan's exhaustion verdict — the commit leaves
`AtlasExhausted` to the surface rather than falling back to a clear,
because a batch that cannot fit even after evicting everything cold is
a real failure the caller must see, not a silent re-lower. `Grow`
still doubles the atlas once per renderer (generation bumps there, as
on dev). `scenes/perf/map-pan` pans the map by a fractional
ease-in-out offset over 4 s: every frame's fraction is a new
`path::placement` key, so each moving frame produces the raster churn
the commit must fit — without the eviction design the atlas would
still clear about every third moving frame and re-rasterize ~85% of
paths. A retained `PathEmit` is re-found through `Atlas::path` and
replayed instead of re-rasterized — the case the issue names — while
the frame's misses are the coldest shelves, so the next commit
reclaims exactly the band the previous frame's churn left behind.
