# Engine capabilities validation

Baseline: `ecf7c8e11c0e0593b3dd9434a217d87820f7ba5f` (`origin/dev`). Validated implementation: `0e6974020804d2dc8018349468ee7384d0564d61`.
The [capability audit](migration-capabilities.md) lists downstream callers, every old-branch disposition and repository adaptations. The [integration contracts](retained-integrations.md) document recording, ownership, color, timing and scheduling.

## Gates

The final combined command ran `merge-gate.sh capabilities-ir2`, actual Rust 1.95 workspace/all-target checks, ten downstream compile checks, and the instruction profiles.

- Rust 1.98.1: formatting, workspace/all-target Clippy with warnings denied, **340 nextest tests**, combined bench-feature Clippy, cargo-hack each-feature checks, and workspace doctests passed.
- MSRV: Rust 1.95.0 workspace/all-target checks passed.
- All ten downstream checks passed, including separate WaterUI graphics GPU/CPU and barcode GPU-export checks. Local path patches and the documented downstream adaptations were used.
- Original **83 scenes**: metric objects and engine PNG bytes exactly match the original dev reference, per scene. FLIP mean remains **0.004523845590774329**. All **88** current-branch corpus reports were also verified.
- Retained/incremental behavior suites, randomized incremental/full comparison, and the new producer, shader, filter, presentation and recording tests passed.

## CPU instruction counts

Callgrind uses the #43 method: instrumentation initially off, 30-second warming (120 seconds for effects), separate threads, and function-return dumps for `GpuRenderer::lower_content` and the benchmark Cherenkov adapter's `Engine::encode`. The first dump is discarded; the table reports the **second steady sample**, with exactly one root call. These are inclusive CPU phase counts, not whole-thread or GPU-driver totals.

The initial candidate and dev also captured third samples. Lower counts repeat exactly on map/chart/text-page; effects/UI-list and some encode samples show allocator/cache/queue-path variation. Those checks are retained in the raw report. The final candidate stops after the requested second samples, using the same warmup and boundaries. No averaging or favorable-sample selection is used.

| Scene | Dev lower Ir | Final lower Ir | Delta | Dev encode Ir | Final encode Ir | Delta |
|---|---:|---:|---:|---:|---:|---:|
| map | 4,549,865 | 4,516,991 | -0.723% | 1,040,399 | 1,039,880 | -0.050% |
| chart | 273,905 | 272,925 | -0.358% | 137,125 | 137,113 | -0.009% |
| text-page | 643,316 | 643,556 | +0.037% | 101,761 | 101,476 | -0.280% |
| ui-list | 1,110,277 | 1,109,493 | -0.071% | 172,724 | 172,555 | -0.098% |
| effects | 230,108 | 229,609 | -0.217% | 21,868 | 21,805 | -0.288% |

Every final delta is at or below the operator's **+1%** regression threshold. The initial instruction-count candidate had map lower **4,598,491 Ir (+1.069%)**. Its solid paint preparation gained 16,000 self instructions across 2,000 calls. Splitting resource/shader preparation from the small inline solid path removed that regression while preserving exact paint and retained-texture semantics.

The final optimization also keeps the ordinary paint payload copyable, common realization/paint helpers inline, and shader-only preparation separate. Retained emissions continue to own exact shader keys; incremental replay does not use frame-relative resource indices.

Binary SHA-256:

- Dev: `df7581fba24ab12136a58f9fa124411d2cb483daed51c1a4cc0ee9f545666aac`
- Final: `1ac31f4bb2a2f8b54dc63e5134dcaf8e9092b839f332c958fe87084fa3af52da`

Event-driven profiling scripts, raw callgrind parts, per-sample metadata and gate logs are retained on the VM in `~/capabilities-results/` (profiles in `ir/`). The final table is `capabilities-ir2-ir.md`; the previous table is `capabilities-ir1-ir.md`. `~/STATUS.md` contains the iteration record.

## Wall-clock handoff

The operator rejected shared-VM native timing as acceptance evidence after identical-binary controls drifted substantially. Those runs are preserved in the work log but support no no-regression claim. Native timing was stopped. The operator will measure wall time on M1 and iPad Pro M4 with the interleaved harness.

This port preserves pre-existing unsupported behavior, including GPU mesh paint and CPU capabilities absent from both engine branches. It does not implement the future Hydrolysis signal-to-layout redesign; it supplies retained, live-operand and engine-timeline contracts for that work.
