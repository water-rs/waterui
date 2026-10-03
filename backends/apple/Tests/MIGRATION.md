# Native backend coverage migration

The 33 former Swift cases below are accounted for individually. The backend has
no Swift implementation or shared C wire API. `Tests/migration.rs` contains the
#299 behavior ports; `Tests/native.rs` retains the separate #294 contract cases.
Both run on the real process main thread under `native-test-support`.

Source migration is not a runtime pass: cloud verification must build and run the
native target and the reference app on the same simulator/runtime. Run
`.github/scripts/prepare-native-reference.sh <artifact-directory>` once and export
its stdout path as `WATERUI_REFERENCE_METRICS`. The target runner forwards it to
the simulator process. Missing metrics fail the tests. The Swift app is only a
live platform reference, never part of the backend implementation. Its output is
generated, not committed; completion comes from native layout callbacks.

## Removed wire contracts (six cases)

| Former case | Disposition |
| --- | --- |
| WuiArrayTests.mapTransformsInPlace | Removed `WuiArray` C allocation/mapping wrapper; native Rust collections cross no wire boundary. |
| WuiArrayTests.wuiStrRoundTripsAString | Removed `WuiStr` C string ownership wrapper. Native `Str` is passed directly. |
| WuiArrayTests.wuiStrRoundTripsUTF8 | Same removed string transport, including its UTF-8 encode/decode pair. |
| WrapperCallTests.callWrapperInvokesTheWrappedClosure | Removed C closure trampoline; handlers are typed Rust closures. |
| GeometryTests.proposalInitFromCGSize | Removed Swift-to-C proposal constructor; native layout uses `ProposalSize` directly. #294 `leaf::mounting_installs_the_intrinsic_measure` checks native measurement delivery. |
| GeometryTests.viewIdIsValueEqualityOverThe128BitKey | Removed Swift wrapper over the C 128-bit type key; there is no replacement transport key in the backend. |

## Signals and ownership (14 cases)

| Former case | Native assertion |
| --- | --- |
| SignalSubscriptionTests.initialValueComesFromReadWhenSubscribeIsSilent | #294 `leaf::bind_applies_now_and_on_every_change`; #299 `signals::independent_owners` observes initial value 5 in both targets before any write. |
| SignalSubscriptionTests.synchronousDeliveryDuringSubscribeStoresWithoutNotifying | The removed Swift subscription state machine supported a C callback firing during registration. Native `Signal::watch` has no Swift initialization/notification phase. `signals::silent_watch` asserts the actual typed registration behavior; `independent_owners` asserts `bind` initializes each target exactly once. |
| SignalSubscriptionTests.updatesFireOnChangeAndUpdateValue | `signals::ordered_updates` asserts synchronous ordered deliveries; `owned_values` asserts the current target value after replacement. |
| SignalSubscriptionTests.disposeValueRunsOnReplaceAndOnDeinit | `signals::owned_values` asserts drop IDs `[1]`, `[1,2]`, then `[1,2,3]` as values are replaced and owners destroyed. |
| SignalSubscriptionTests.cancelDropsTheWatcher | `signals::independent_owners` asserts one capture destructor after repeated owner removal and no later callback to that owner. |
| ComputedTests.valueReadsThrough | #294 `leaf::bind_applies_now_and_on_every_change` and #299 `signals::owned_values` assert the initial snapshot reaches the target. The removed C `read` call-count has no native ABI equivalent. |
| ComputedTests.watchDeliversUpdates | `signals::ordered_updates` asserts each value immediately after its write and the complete sequence. |
| ComputedTests.deinitDropsInner | The opaque C computed handle was removed. `signals::owned_values` asserts target retention during binding, release on leaf drop, and final value release after source drop. |
| ComputedTests.observationMirrorsValueAndFiresOnChange | `signals::owned_values` checks stored target values; `independent_owners` checks initial and subsequent notification sequences. |
| WatcherGuardTests.cancelRunsHandlerExactlyOnce | `signals::independent_owners` removes an optional leaf owner twice and asserts exactly one capture destructor. Cancellation is ownership consumption in Rust. |
| WatcherGuardTests.deinitCancels | `signals::drop_cancels` writes after leaf destruction and asserts no delivery. |
| ReactiveWatcherListTests.notifyCallsEveryWatcherWithTheCurrentValue | `signals::independent_owners` asserts both owners receive 5 then 9. |
| ReactiveWatcherListTests.removeWatcherReleasesExactlyOnce | `signals::independent_owners` asserts destructor sequence `[1]` then `[1,2]` when owners are removed independently. |
| ReactiveWatcherListTests.removedWatcherIsNotNotified | `signals::independent_owners` writes 12 after removing the first owner; only the second sequence changes. |

## Color (seven cases)

| Former case | Native assertion |
| --- | --- |
| ColorConversionTests.srgbToLinearKnownValues | `color::srgb_known_values` checks 0, 1, 0.5 and the transfer-function breakpoint against the original numeric expectations. |
| NSColorConversionTests.extendedRangeComponentsSurvive | `color::native_hdr_and_updates` reads the actual AppKit leaf's layer color, asserts extended linear Display-P3, four straight components, negative and HDR channel preservation, and reactive replacement. |
| NSColorConversionTests.sdrConversionClampsToUnitRange | Removed generic Swift `toNSColor(allowHdr:false)` contract. Working-color native fills preserve HDR; `native_hdr_and_updates` explicitly asserts channels are not clamped. This does not remove a component-specific SDR selection policy. |
| NSColorConversionTests.resolvedColorRoundTripsThroughNSColor | `color::native_well_round_trip` renders the actual color picker, sends its installed native action, and compares the resulting binding's resolved RGBA to the input within 0.001. |
| UIColorConversionTests.extendedRangeComponentsSurvive | `color::native_hdr_and_updates` reads the actual UIKit leaf's background CGColor, asserts its Display-P3 space and all four channels. |
| UIColorConversionTests.hdrChannelsRemainUnscaled | The same test updates to `[1.4,0.25,0.125,0.5]` and checks every channel without headroom multiplication or alpha premultiplication. |
| UIColorConversionTests.sdrConversionClampsToUnitRange | Removed generic Swift `toUIColor(allowHdr:false)` contract, as for AppKit; native fill assertions require unclamped HDR. |

## Hosted UIKit behavior (six cases)

| Former case | Native assertion |
| --- | --- |
| TextFieldPlainMetricsTests.testPlainInputMatchesSwiftUIAutomaticFieldHeight | `uikit::plain_field_matches_swiftui`: actual field text `x`, empty label, width 402; measured height equals live `UIHostingController(TextField)` within 0.5pt. Border style, zero layer border, transparent fill, and zero text/editing rect insets are independently asserted. |
| ListRowInsetsTests.testRowInsetsMatchDisplayedCellMargins | `uikit::list_chrome_matches_swiftui` compares all four actual mounted row-content insets to a displayed stock inset-grouped table cell's directional margins within 0.5pt. |
| ListRowInsetsTests.testRowPitchMatchesHostedSwiftUI | The same test renders red content of height 24 and compares its actual native cell height with live SwiftUI List within 0.5pt. |
| ListRowInsetsTests.testMinimumRowHeightMatchesHostedSwiftUI | The same test renders height 4 without a minimum override and compares its actual cell height with the live SwiftUI minimum within 0.5pt. |
| ListCellLayoutTests.testNestedStackTextHasNonZeroFramesInsideCells | `uikit::nested_list_frames` examines every visible cell and every nonempty UILabel; positive width/height, intersection with the cell expanded by 1pt, nonzero cells, and at least three text views. |
| CompactSplitTests.testCollapsedSplitShowsSidebar | `uikit::compact_split` starts with selection 1, requires a collapsed split and visible primary controller, and asserts an optional secondary controller is not visible without forcing its view to load. |

`Support.swift` and `DeviceHostedApp.swift` supported the removed test engine;
neither contained a separate test case. Native UIKit cases use the production
`embedding::mount_content` path through the feature-gated internal support
module. It includes primary-content forwarding and controller containment. Tests
do not force an extra root layout pass: production placement completes the
child's layout after delivering its new proposal and frame.
