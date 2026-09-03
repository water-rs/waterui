window.BENCHMARK_DATA = {
  "lastUpdate": 1788434076049,
  "repoUrl": "https://github.com/water-rs/waterui",
  "entries": {
    "WaterUI Bench (ubuntu-latest)": [
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "5e8aa7974f52d1b2a0faaa13463af3df32c20d34",
          "message": "ci: let coverage report every failure instead of stopping at the first\n\nThe coverage job ran nextest under its default profile, so fail-fast was\non: the run stopped at the first failing test and 632 of 1616 tests\nnever executed, hiding their state behind one failure (#153).\n\nIt now runs under the ci profile via NEXTEST_PROFILE, since cargo-llvm-cov\nreads a --profile flag as cargo's build profile rather than passing it\nthrough to nextest.",
          "timestamp": "2026-08-20T14:20:34-04:00",
          "tree_id": "f5baa355681468269ab4012ca17ea54540311ac6",
          "url": "https://github.com/water-rs/waterui/commit/5e8aa7974f52d1b2a0faaa13463af3df32c20d34"
        },
        "date": 1787252666220,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 347133,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 338334,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 27234,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 23540,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "fee088a9397dd9d2dbe05abd680f6ddfd10e0c0f",
          "message": "ci: record benchmark history for the platforms that produced results\n\nA bench leg that fails uploads no JSON, so the history job errored on the\nmissing file and discarded the runs that had succeeded. Windows is\ndeliberately non-gating until #152 is resolved, so that is the expected\nsteady state, not an exception.\n\nEach platform's history step now runs only when its result file exists.\nmacOS and Linux history is recorded either way; a broken platform costs\nits own series and nobody else's.",
          "timestamp": "2026-08-20T15:22:00-04:00",
          "tree_id": "0c4d547b95cc4712e8aaf2cef8ca3c3ad9347790",
          "url": "https://github.com/water-rs/waterui/commit/fee088a9397dd9d2dbe05abd680f6ddfd10e0c0f"
        },
        "date": 1787256172572,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 418210,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 409961,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 29175,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 26265,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "f14490d9eb1ef1b62f480d93886e00aa70a7e588",
          "message": "fix(browser-wpe): drop the dependency entries a CI image cannot satisfy\n\nThe first real run of the engine build got as far as installing WPE's\ndependencies and died there with exit 100, on ubuntu-22.04:\n\n    git-svn : Depends: git (< 1:2.34.1-.) but 1:2.55.0-0ppa1~ubuntu22.04.2\n    libgstreamer1.0-dev : Depends: libunwind-dev\n    E: Unable to correct problems, you have held broken packages\n\nGitHub's runner images install git from a PPA, so 22.04's git-svn can\nnever be satisfied there — and git-svn is tooling for the SVN workflow\nWebKit has retired, which nothing in this build reads. One unusable\nconvenience package was aborting the entire install.\n\nThis is the third blocker in the same script, after the missing sourced\nfile and the libbacktrace requirement, and it applies equally to the\nexisting browser-runtime-wpe job, which runs the same script on the same\nrunner. That job has never produced an artifact.",
          "timestamp": "2026-08-21T04:48:37-04:00",
          "tree_id": "12afe093f09feb6c19f47706e883cbd78c58509e",
          "url": "https://github.com/water-rs/waterui/commit/f14490d9eb1ef1b62f480d93886e00aa70a7e588"
        },
        "date": 1787304054725,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 451301,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 441538,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 35387,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 32577,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "37947d28099f2d9fce17fb08574ac66822bbb2d6",
          "message": "ci: stop the WPE workflow from failing on every webview change\n\nThe engine build cannot succeed today: ubuntu-22.04 ships GCC 11.4 and\nWebKit 2.52.5 requires 12.2, while source.toml's maximum_glibc of 2.35\nis exactly that image's glibc, so the build cannot move to a newer runner\nwithout giving up a portable artifact. Issue #155 records that and the\nthree blockers already fixed.\n\nA guaranteed red X on every webview change is worse than no signal — it\nis a signal everyone learns to ignore. Dispatch remains so the path stays\nexercisable; the weekly schedule goes with the path triggers, since it\nexisted to keep a runtime cache warm that no successful build has ever\npopulated. Both come back in the change that makes the build work.",
          "timestamp": "2026-08-21T05:49:33-04:00",
          "tree_id": "5839b2857c6d92d57f3875b0347f6d2d83431da5",
          "url": "https://github.com/water-rs/waterui/commit/37947d28099f2d9fce17fb08574ac66822bbb2d6"
        },
        "date": 1787307648769,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 470740,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 439792,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 37883,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 32727,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "2434e0ea9d7faa2c0bfae0f074b8669a16e800da",
          "message": "fix(str): make Debug print the string instead of the tagged representation\n\nStr derived Debug on a struct whose fields are a raw pointer and a length\nthat is negative whenever the string is owned, so {:?} produced\nStr { ptr: .., len: -9 }. Every other string type prints its contents,\nand this is unreadable exactly where Debug earns its keep: assertion\nfailures, tracing fields, and the {:?} of any type that holds one. It\ncost a diagnostic round trip while writing the CEF real-engine tests.\n\nNow delegates to str, with a test pinning both representations so the\nderive cannot come back.",
          "timestamp": "2026-08-21T19:31:40-04:00",
          "tree_id": "b46cea4c6bed30d3c878119495f79e96309b458f",
          "url": "https://github.com/water-rs/waterui/commit/2434e0ea9d7faa2c0bfae0f074b8669a16e800da"
        },
        "date": 1787358485827,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 383857,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 375629,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 26151,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 23367,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "1bd764b37f114b80c6dcb0a33b157a10664b308a",
          "message": "fix(cli): repin the apple backend to the current submodule\n\nThe GPU feature-pruning commit advanced the Apple submodule; the scaffold\npin follows, as the build_info guard test demands.",
          "timestamp": "2026-08-21T21:34:04-04:00",
          "tree_id": "1305fdf34ffa968815dbed30b9dc3d77ab1fe28d",
          "url": "https://github.com/water-rs/waterui/commit/1bd764b37f114b80c6dcb0a33b157a10664b308a"
        },
        "date": 1787368816046,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 462376,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 453715,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 41442,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 36458,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "1137964cc408a463bc55be3ea2e7de0060cb1be1",
          "message": "fix(ci): gate the 10-bit visual on adapter capability and unblock two red jobs\n\nThree independent reds from run 32550974672. The Coverage job's adapter\nlacks TEXTURE_FORMAT_16BIT_NORM, so the P010 color-visual export died in\nDevice::create_texture; the 10-bit visual is genuinely unrenderable there\n(the documented required_media_features contract), so the test now skips\nthat one export with a warning instead of asking wgpu for a texture the\ndevice cannot carry. The Linux-only browser-wpe lint failure wanted a\nPanics section on DmaBufFrame::with_visible_size; documented. The FFI\nHeader job finished its work but was killed by its own 30-minute cap on a\ncache-miss day (30m21s); raised to 45. Also records the no-silent-waits\nrule in AGENTS.md: waiters emit a heartbeat or re-arm within ~50 minutes\nbecause the prompt cache TTL is one hour.",
          "timestamp": "2026-08-22T01:11:36-04:00",
          "tree_id": "eaf59144c927241fc0785a4f44bc3074a2fa2262",
          "url": "https://github.com/water-rs/waterui/commit/1137964cc408a463bc55be3ea2e7de0060cb1be1"
        },
        "date": 1787376966772,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 594870,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 543409,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 41749,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 37148,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "e436ee0db0e42bba0e43754fc1dc26540e80ac5f",
          "message": "style(graphics): wrap the scene-engine capability doc to rustfmt's liking",
          "timestamp": "2026-08-22T06:18:15-04:00",
          "tree_id": "a85823a55c4147291e38578e55c020a7b97c8cf2",
          "url": "https://github.com/water-rs/waterui/commit/e436ee0db0e42bba0e43754fc1dc26540e80ac5f"
        },
        "date": 1787397723398,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 444620,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 439443,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 34365,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 31479,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "04ed5ab566eafcc707f7f56eb8ffb969e16fbd18",
          "message": "chore(cli): repin the android backend to the split-pane fix",
          "timestamp": "2026-08-23T06:14:50Z",
          "url": "https://github.com/water-rs/waterui/commit/04ed5ab566eafcc707f7f56eb8ffb969e16fbd18"
        },
        "date": 1787468693483,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 456985,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 448014,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 39154,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 35882,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "f63a8849bf2789abc192ca3124f806de0172f5b4",
          "message": "chore(ffi): regenerate the C header after the doc-comment cleanup\n\ncbindgen carries Rust doc comments straight into `waterui.h`, so\nbackticking the generics in them — done so rustdoc would stop reading\n`Metadata<Environment>` as an unclosed HTML tag — changed the generated\nheader too. All three copies move together, and the pins follow.\n\nThe header check is what caught this, which is exactly its job: the\nrustdoc cleanup edited five files under `ffi/` and I did not regenerate.",
          "timestamp": "2026-08-24T07:17:34Z",
          "url": "https://github.com/water-rs/waterui/commit/f63a8849bf2789abc192ca3124f806de0172f5b4"
        },
        "date": 1787557311061,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 462646,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 457408,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 39106,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 35314,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "611d807a69a841dfe6dd7aab650deb1aa669f666",
          "message": "fix(hydrolysis): reclaim GPU memory after the renderer is gone, not before\n\nThe previous attempt put the reclaim in `OffscreenSurface`'s own drop, where\nit cannot do its job: `RuntimeWindow` declares its platform window before its\nrenderer, fields drop in declaration order, so the surface goes first and the\npoll runs while Vello still holds every pipeline and buffer it allocated.\nWindows kept running out of memory in the perf probe, which builds and drops\n278 runtimes on one device.\n\n`OffscreenGpuContext::reclaim` is now explicit, and `HeadlessRuntime` holds a\nguard declared after everything that owns GPU resources — the same reason\n`_executor_teardown` sits where it does — so the device is asked to release a\nruntime's allocations once that runtime is entirely gone. `TestHost::render`\ndrops its renderer and window and reclaims before returning, so a host that\nrenders repeatedly does not accumulate either.\n\nThe surface keeps its own reclaim for a bare surface with no renderer above\nit; that is all it can see from there.\n\nWindows was down to this one test: 1683 passed, 1 failed. Locally 41 tests\npass and the probe reports the same ratios in 22s.",
          "timestamp": "2026-08-25T04:39:57Z",
          "url": "https://github.com/water-rs/waterui/commit/611d807a69a841dfe6dd7aab650deb1aa669f666"
        },
        "date": 1787642680055,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 448189,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 443415,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 38067,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 33610,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "cca2121dd858429296cbac21a77c9ea5b9d023b8",
          "message": "ci: cap how many wgpu devices Windows creates at once\n\n`selected_tooltip_exposes_accessibility_labels` failed in `request_device`\nwith `Core(Device(OutOfMemory))`, exhausting all three retries, and its\nneighbours in the same file were reported flaky in the same run — passing\nonly on a retry. Tests that fail when run beside others and pass when run\nagain are not broken tests; they are tests at a resource limit.\n\nThe resource is wgpu devices. Nearly every test package mounts a\n`waterui-testing` host and each host requests its own device, so nextest's\ndefault of one test per core means four live devices. `windows-latest` has\nno GPU, so all four are WARP devices sharing the machine's 16 GB, and the\nsuite ran out.\n\nCapped at two on Windows through a test group, which is the only way to\nexpress this per-platform — `test-threads` is profile-wide. Every test still\nruns; the peak number of live WARP devices halves. Confirmed inert\nelsewhere: `nextest show-config test-groups` reports \"(no matches)\" on\nmacOS, and the Linux runners rasterize on llvmpipe, which is far cheaper per\ndevice.\n\nThis lengthens the Windows job. That is the cost of a runner with no GPU,\nand it buys a result that means something.",
          "timestamp": "2026-08-25T17:41:55Z",
          "url": "https://github.com/water-rs/waterui/commit/cca2121dd858429296cbac21a77c9ea5b9d023b8"
        },
        "date": 1787729079452,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 468813,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 454117,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 40384,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 35690,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "cf9ad72aac303f6a7eab0ea0b8c695e93184f16d",
          "message": "chore: record the WaterKit licence texts",
          "timestamp": "2026-08-27T18:03:01Z",
          "url": "https://github.com/water-rs/waterui/commit/cf9ad72aac303f6a7eab0ea0b8c695e93184f16d"
        },
        "date": 1787853872578,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 349818,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 329477,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 20400,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 19320,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "57535ef91f8aa0d740ea67420e967c3f0566b3f4",
          "message": "fix(browser-cef): document the Windows sandbox unsafe blocks\n\nThe Windows-only cfg block was never compiled by the macOS lint passes,\nso these three calls escaped the CEF C-ABI safety-comment cleanup; the\nWindows workspace clippy leg now compiles browser-cef through the C1\nexample dependencies and rejects them.\n\nClaude-Session: https://claude.ai/code/session_01XwLTWGKnqhKDu4ym3qEobm",
          "timestamp": "2026-08-28T12:03:47Z",
          "url": "https://github.com/water-rs/waterui/commit/57535ef91f8aa0d740ea67420e967c3f0566b3f4"
        },
        "date": 1787943104487,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 346099,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 324275,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 23860,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 19891,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788008293219,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 370114,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 364978,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 23414,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 22259,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788088409686,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 464664,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 453358,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 40739,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 34371,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788178971888,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 463610,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 453066,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 39579,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 35294,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788260327983,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 462021,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 446654,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 39772,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 36273,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ce31ee685c242b052555d89dd8b064c40f3eba95",
          "message": "Merge pull request #242 from water-rs/agent/hydrolysis-scene-engine-per-adapter-v2\n\nfix(hydrolysis): keep materialized views out of the address-keyed measure cache",
          "timestamp": "2026-09-02T09:22:39Z",
          "url": "https://github.com/water-rs/waterui/commit/ce31ee685c242b052555d89dd8b064c40f3eba95"
        },
        "date": 1788348298544,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 354479,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 329902,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 21205,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 19264,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4623c37038814dbc2d2314061a32b7a968b8980d",
          "message": "Merge pull request #293 from water-rs/agent/kit-pin-waterkit-dev\n\nbuild(kit): pin waterkit at dev, where the tracked-and-ignored Info.plist is fixed",
          "timestamp": "2026-09-03T09:58:00Z",
          "url": "https://github.com/water-rs/waterui/commit/4623c37038814dbc2d2314061a32b7a968b8980d"
        },
        "date": 1788434071109,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 452903,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 439517,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 33174,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 31960,
            "unit": "us"
          }
        ]
      }
    ],
    "WaterUI Bench (macos-latest)": [
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "5e8aa7974f52d1b2a0faaa13463af3df32c20d34",
          "message": "ci: let coverage report every failure instead of stopping at the first\n\nThe coverage job ran nextest under its default profile, so fail-fast was\non: the run stopped at the first failing test and 632 of 1616 tests\nnever executed, hiding their state behind one failure (#153).\n\nIt now runs under the ci profile via NEXTEST_PROFILE, since cargo-llvm-cov\nreads a --profile flag as cargo's build profile rather than passing it\nthrough to nextest.",
          "timestamp": "2026-08-20T14:20:34-04:00",
          "tree_id": "f5baa355681468269ab4012ca17ea54540311ac6",
          "url": "https://github.com/water-rs/waterui/commit/5e8aa7974f52d1b2a0faaa13463af3df32c20d34"
        },
        "date": 1787252669329,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 697479,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 546564,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 15980,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 12572,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "fee088a9397dd9d2dbe05abd680f6ddfd10e0c0f",
          "message": "ci: record benchmark history for the platforms that produced results\n\nA bench leg that fails uploads no JSON, so the history job errored on the\nmissing file and discarded the runs that had succeeded. Windows is\ndeliberately non-gating until #152 is resolved, so that is the expected\nsteady state, not an exception.\n\nEach platform's history step now runs only when its result file exists.\nmacOS and Linux history is recorded either way; a broken platform costs\nits own series and nobody else's.",
          "timestamp": "2026-08-20T15:22:00-04:00",
          "tree_id": "0c4d547b95cc4712e8aaf2cef8ca3c3ad9347790",
          "url": "https://github.com/water-rs/waterui/commit/fee088a9397dd9d2dbe05abd680f6ddfd10e0c0f"
        },
        "date": 1787256174897,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 766946,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 523362,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 22823,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 15876,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "f14490d9eb1ef1b62f480d93886e00aa70a7e588",
          "message": "fix(browser-wpe): drop the dependency entries a CI image cannot satisfy\n\nThe first real run of the engine build got as far as installing WPE's\ndependencies and died there with exit 100, on ubuntu-22.04:\n\n    git-svn : Depends: git (< 1:2.34.1-.) but 1:2.55.0-0ppa1~ubuntu22.04.2\n    libgstreamer1.0-dev : Depends: libunwind-dev\n    E: Unable to correct problems, you have held broken packages\n\nGitHub's runner images install git from a PPA, so 22.04's git-svn can\nnever be satisfied there — and git-svn is tooling for the SVN workflow\nWebKit has retired, which nothing in this build reads. One unusable\nconvenience package was aborting the entire install.\n\nThis is the third blocker in the same script, after the missing sourced\nfile and the libbacktrace requirement, and it applies equally to the\nexisting browser-runtime-wpe job, which runs the same script on the same\nrunner. That job has never produced an artifact.",
          "timestamp": "2026-08-21T04:48:37-04:00",
          "tree_id": "12afe093f09feb6c19f47706e883cbd78c58509e",
          "url": "https://github.com/water-rs/waterui/commit/f14490d9eb1ef1b62f480d93886e00aa70a7e588"
        },
        "date": 1787304057152,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 684849,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 564734,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 24317,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 16268,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "37947d28099f2d9fce17fb08574ac66822bbb2d6",
          "message": "ci: stop the WPE workflow from failing on every webview change\n\nThe engine build cannot succeed today: ubuntu-22.04 ships GCC 11.4 and\nWebKit 2.52.5 requires 12.2, while source.toml's maximum_glibc of 2.35\nis exactly that image's glibc, so the build cannot move to a newer runner\nwithout giving up a portable artifact. Issue #155 records that and the\nthree blockers already fixed.\n\nA guaranteed red X on every webview change is worse than no signal — it\nis a signal everyone learns to ignore. Dispatch remains so the path stays\nexercisable; the weekly schedule goes with the path triggers, since it\nexisted to keep a runtime cache warm that no successful build has ever\npopulated. Both come back in the change that makes the build work.",
          "timestamp": "2026-08-21T05:49:33-04:00",
          "tree_id": "5839b2857c6d92d57f3875b0347f6d2d83431da5",
          "url": "https://github.com/water-rs/waterui/commit/37947d28099f2d9fce17fb08574ac66822bbb2d6"
        },
        "date": 1787307650753,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 675384,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 512698,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 16916,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 13310,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "2434e0ea9d7faa2c0bfae0f074b8669a16e800da",
          "message": "fix(str): make Debug print the string instead of the tagged representation\n\nStr derived Debug on a struct whose fields are a raw pointer and a length\nthat is negative whenever the string is owned, so {:?} produced\nStr { ptr: .., len: -9 }. Every other string type prints its contents,\nand this is unreadable exactly where Debug earns its keep: assertion\nfailures, tracing fields, and the {:?} of any type that holds one. It\ncost a diagnostic round trip while writing the CEF real-engine tests.\n\nNow delegates to str, with a test pinning both representations so the\nderive cannot come back.",
          "timestamp": "2026-08-21T19:31:40-04:00",
          "tree_id": "b46cea4c6bed30d3c878119495f79e96309b458f",
          "url": "https://github.com/water-rs/waterui/commit/2434e0ea9d7faa2c0bfae0f074b8669a16e800da"
        },
        "date": 1787358488133,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 713145,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 551368,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 22427,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 15876,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "1bd764b37f114b80c6dcb0a33b157a10664b308a",
          "message": "fix(cli): repin the apple backend to the current submodule\n\nThe GPU feature-pruning commit advanced the Apple submodule; the scaffold\npin follows, as the build_info guard test demands.",
          "timestamp": "2026-08-21T21:34:04-04:00",
          "tree_id": "1305fdf34ffa968815dbed30b9dc3d77ab1fe28d",
          "url": "https://github.com/water-rs/waterui/commit/1bd764b37f114b80c6dcb0a33b157a10664b308a"
        },
        "date": 1787368818468,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 704136,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 572847,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 19556,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 16020,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "1137964cc408a463bc55be3ea2e7de0060cb1be1",
          "message": "fix(ci): gate the 10-bit visual on adapter capability and unblock two red jobs\n\nThree independent reds from run 32550974672. The Coverage job's adapter\nlacks TEXTURE_FORMAT_16BIT_NORM, so the P010 color-visual export died in\nDevice::create_texture; the 10-bit visual is genuinely unrenderable there\n(the documented required_media_features contract), so the test now skips\nthat one export with a warning instead of asking wgpu for a texture the\ndevice cannot carry. The Linux-only browser-wpe lint failure wanted a\nPanics section on DmaBufFrame::with_visible_size; documented. The FFI\nHeader job finished its work but was killed by its own 30-minute cap on a\ncache-miss day (30m21s); raised to 45. Also records the no-silent-waits\nrule in AGENTS.md: waiters emit a heartbeat or re-arm within ~50 minutes\nbecause the prompt cache TTL is one hour.",
          "timestamp": "2026-08-22T01:11:36-04:00",
          "tree_id": "eaf59144c927241fc0785a4f44bc3074a2fa2262",
          "url": "https://github.com/water-rs/waterui/commit/1137964cc408a463bc55be3ea2e7de0060cb1be1"
        },
        "date": 1787376969497,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 583060,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 454977,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 14101,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 11871,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "committer": {
            "email": "me@lexo.cool",
            "name": "Lexo Liu",
            "username": "lexoliu"
          },
          "distinct": true,
          "id": "e436ee0db0e42bba0e43754fc1dc26540e80ac5f",
          "message": "style(graphics): wrap the scene-engine capability doc to rustfmt's liking",
          "timestamp": "2026-08-22T06:18:15-04:00",
          "tree_id": "a85823a55c4147291e38578e55c020a7b97c8cf2",
          "url": "https://github.com/water-rs/waterui/commit/e436ee0db0e42bba0e43754fc1dc26540e80ac5f"
        },
        "date": 1787397725598,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 541779,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 453571,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 15700,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 14078,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "04ed5ab566eafcc707f7f56eb8ffb969e16fbd18",
          "message": "chore(cli): repin the android backend to the split-pane fix",
          "timestamp": "2026-08-23T06:14:50Z",
          "url": "https://github.com/water-rs/waterui/commit/04ed5ab566eafcc707f7f56eb8ffb969e16fbd18"
        },
        "date": 1787468695830,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 650091,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 537027,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 20726,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 15428,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "f63a8849bf2789abc192ca3124f806de0172f5b4",
          "message": "chore(ffi): regenerate the C header after the doc-comment cleanup\n\ncbindgen carries Rust doc comments straight into `waterui.h`, so\nbackticking the generics in them — done so rustdoc would stop reading\n`Metadata<Environment>` as an unclosed HTML tag — changed the generated\nheader too. All three copies move together, and the pins follow.\n\nThe header check is what caught this, which is exactly its job: the\nrustdoc cleanup edited five files under `ffi/` and I did not regenerate.",
          "timestamp": "2026-08-24T07:17:34Z",
          "url": "https://github.com/water-rs/waterui/commit/f63a8849bf2789abc192ca3124f806de0172f5b4"
        },
        "date": 1787557314227,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 1054892,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 674588,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 18807,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 14905,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "611d807a69a841dfe6dd7aab650deb1aa669f666",
          "message": "fix(hydrolysis): reclaim GPU memory after the renderer is gone, not before\n\nThe previous attempt put the reclaim in `OffscreenSurface`'s own drop, where\nit cannot do its job: `RuntimeWindow` declares its platform window before its\nrenderer, fields drop in declaration order, so the surface goes first and the\npoll runs while Vello still holds every pipeline and buffer it allocated.\nWindows kept running out of memory in the perf probe, which builds and drops\n278 runtimes on one device.\n\n`OffscreenGpuContext::reclaim` is now explicit, and `HeadlessRuntime` holds a\nguard declared after everything that owns GPU resources — the same reason\n`_executor_teardown` sits where it does — so the device is asked to release a\nruntime's allocations once that runtime is entirely gone. `TestHost::render`\ndrops its renderer and window and reclaims before returning, so a host that\nrenders repeatedly does not accumulate either.\n\nThe surface keeps its own reclaim for a bare surface with no renderer above\nit; that is all it can see from there.\n\nWindows was down to this one test: 1683 passed, 1 failed. Locally 41 tests\npass and the probe reports the same ratios in 22s.",
          "timestamp": "2026-08-25T04:39:57Z",
          "url": "https://github.com/water-rs/waterui/commit/611d807a69a841dfe6dd7aab650deb1aa669f666"
        },
        "date": 1787642682613,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 945214,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 591835,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 23568,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 16968,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "cca2121dd858429296cbac21a77c9ea5b9d023b8",
          "message": "ci: cap how many wgpu devices Windows creates at once\n\n`selected_tooltip_exposes_accessibility_labels` failed in `request_device`\nwith `Core(Device(OutOfMemory))`, exhausting all three retries, and its\nneighbours in the same file were reported flaky in the same run — passing\nonly on a retry. Tests that fail when run beside others and pass when run\nagain are not broken tests; they are tests at a resource limit.\n\nThe resource is wgpu devices. Nearly every test package mounts a\n`waterui-testing` host and each host requests its own device, so nextest's\ndefault of one test per core means four live devices. `windows-latest` has\nno GPU, so all four are WARP devices sharing the machine's 16 GB, and the\nsuite ran out.\n\nCapped at two on Windows through a test group, which is the only way to\nexpress this per-platform — `test-threads` is profile-wide. Every test still\nruns; the peak number of live WARP devices halves. Confirmed inert\nelsewhere: `nextest show-config test-groups` reports \"(no matches)\" on\nmacOS, and the Linux runners rasterize on llvmpipe, which is far cheaper per\ndevice.\n\nThis lengthens the Windows job. That is the cost of a runner with no GPU,\nand it buys a result that means something.",
          "timestamp": "2026-08-25T17:41:55Z",
          "url": "https://github.com/water-rs/waterui/commit/cca2121dd858429296cbac21a77c9ea5b9d023b8"
        },
        "date": 1787729082039,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 659033,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 505557,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 20485,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 14676,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "cf9ad72aac303f6a7eab0ea0b8c695e93184f16d",
          "message": "chore: record the WaterKit licence texts",
          "timestamp": "2026-08-27T18:03:01Z",
          "url": "https://github.com/water-rs/waterui/commit/cf9ad72aac303f6a7eab0ea0b8c695e93184f16d"
        },
        "date": 1787853875139,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 789356,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 595699,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 30740,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 20731,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "57535ef91f8aa0d740ea67420e967c3f0566b3f4",
          "message": "fix(browser-cef): document the Windows sandbox unsafe blocks\n\nThe Windows-only cfg block was never compiled by the macOS lint passes,\nso these three calls escaped the CEF C-ABI safety-comment cleanup; the\nWindows workspace clippy leg now compiles browser-cef through the C1\nexample dependencies and rejects them.\n\nClaude-Session: https://claude.ai/code/session_01XwLTWGKnqhKDu4ym3qEobm",
          "timestamp": "2026-08-28T12:03:47Z",
          "url": "https://github.com/water-rs/waterui/commit/57535ef91f8aa0d740ea67420e967c3f0566b3f4"
        },
        "date": 1787943107704,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 928130,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 629865,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 21256,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 16078,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788008296544,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 862402,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 591087,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 26125,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 17965,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788088413438,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 912177,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 619075,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 18708,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 14558,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788178976384,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 1102022,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 673375,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 27467,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 18341,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788260332205,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 575103,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 493072,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 23237,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 16006,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ce31ee685c242b052555d89dd8b064c40f3eba95",
          "message": "Merge pull request #242 from water-rs/agent/hydrolysis-scene-engine-per-adapter-v2\n\nfix(hydrolysis): keep materialized views out of the address-keyed measure cache",
          "timestamp": "2026-09-02T09:22:39Z",
          "url": "https://github.com/water-rs/waterui/commit/ce31ee685c242b052555d89dd8b064c40f3eba95"
        },
        "date": 1788348301434,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 1188943,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 737629,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 25468,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 17700,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "4623c37038814dbc2d2314061a32b7a968b8980d",
          "message": "Merge pull request #293 from water-rs/agent/kit-pin-waterkit-dev\n\nbuild(kit): pin waterkit at dev, where the tracked-and-ignored Info.plist is fixed",
          "timestamp": "2026-09-03T09:58:00Z",
          "url": "https://github.com/water-rs/waterui/commit/4623c37038814dbc2d2314061a32b7a968b8980d"
        },
        "date": 1788434075189,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 709635,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 564816,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 23744,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 16138,
            "unit": "us"
          }
        ]
      }
    ],
    "WaterUI Bench (windows-latest)": [
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "f63a8849bf2789abc192ca3124f806de0172f5b4",
          "message": "chore(ffi): regenerate the C header after the doc-comment cleanup\n\ncbindgen carries Rust doc comments straight into `waterui.h`, so\nbackticking the generics in them — done so rustdoc would stop reading\n`Metadata<Environment>` as an unclosed HTML tag — changed the generated\nheader too. All three copies move together, and the pins follow.\n\nThe header check is what caught this, which is exactly its job: the\nrustdoc cleanup edited five files under `ffi/` and I did not regenerate.",
          "timestamp": "2026-08-24T07:17:34Z",
          "url": "https://github.com/water-rs/waterui/commit/f63a8849bf2789abc192ca3124f806de0172f5b4"
        },
        "date": 1787557317966,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 7839174,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6763646,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 111696,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 54865,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "611d807a69a841dfe6dd7aab650deb1aa669f666",
          "message": "fix(hydrolysis): reclaim GPU memory after the renderer is gone, not before\n\nThe previous attempt put the reclaim in `OffscreenSurface`'s own drop, where\nit cannot do its job: `RuntimeWindow` declares its platform window before its\nrenderer, fields drop in declaration order, so the surface goes first and the\npoll runs while Vello still holds every pipeline and buffer it allocated.\nWindows kept running out of memory in the perf probe, which builds and drops\n278 runtimes on one device.\n\n`OffscreenGpuContext::reclaim` is now explicit, and `HeadlessRuntime` holds a\nguard declared after everything that owns GPU resources — the same reason\n`_executor_teardown` sits where it does — so the device is asked to release a\nruntime's allocations once that runtime is entirely gone. `TestHost::render`\ndrops its renderer and window and reclaims before returning, so a host that\nrenders repeatedly does not accumulate either.\n\nThe surface keeps its own reclaim for a bare surface with no renderer above\nit; that is all it can see from there.\n\nWindows was down to this one test: 1683 passed, 1 failed. Locally 41 tests\npass and the probe reports the same ratios in 22s.",
          "timestamp": "2026-08-25T04:39:57Z",
          "url": "https://github.com/water-rs/waterui/commit/611d807a69a841dfe6dd7aab650deb1aa669f666"
        },
        "date": 1787642684810,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 7852057,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6967028,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 86124,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 50033,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "cca2121dd858429296cbac21a77c9ea5b9d023b8",
          "message": "ci: cap how many wgpu devices Windows creates at once\n\n`selected_tooltip_exposes_accessibility_labels` failed in `request_device`\nwith `Core(Device(OutOfMemory))`, exhausting all three retries, and its\nneighbours in the same file were reported flaky in the same run — passing\nonly on a retry. Tests that fail when run beside others and pass when run\nagain are not broken tests; they are tests at a resource limit.\n\nThe resource is wgpu devices. Nearly every test package mounts a\n`waterui-testing` host and each host requests its own device, so nextest's\ndefault of one test per core means four live devices. `windows-latest` has\nno GPU, so all four are WARP devices sharing the machine's 16 GB, and the\nsuite ran out.\n\nCapped at two on Windows through a test group, which is the only way to\nexpress this per-platform — `test-threads` is profile-wide. Every test still\nruns; the peak number of live WARP devices halves. Confirmed inert\nelsewhere: `nextest show-config test-groups` reports \"(no matches)\" on\nmacOS, and the Linux runners rasterize on llvmpipe, which is far cheaper per\ndevice.\n\nThis lengthens the Windows job. That is the cost of a runner with no GPU,\nand it buys a result that means something.",
          "timestamp": "2026-08-25T17:41:55Z",
          "url": "https://github.com/water-rs/waterui/commit/cca2121dd858429296cbac21a77c9ea5b9d023b8"
        },
        "date": 1787729084873,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 8224312,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 7086002,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 66695,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 46768,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "cf9ad72aac303f6a7eab0ea0b8c695e93184f16d",
          "message": "chore: record the WaterKit licence texts",
          "timestamp": "2026-08-27T18:03:01Z",
          "url": "https://github.com/water-rs/waterui/commit/cf9ad72aac303f6a7eab0ea0b8c695e93184f16d"
        },
        "date": 1787853879043,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 8144387,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 7415344,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 55654,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 41346,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "57535ef91f8aa0d740ea67420e967c3f0566b3f4",
          "message": "fix(browser-cef): document the Windows sandbox unsafe blocks\n\nThe Windows-only cfg block was never compiled by the macOS lint passes,\nso these three calls escaped the CEF C-ABI safety-comment cleanup; the\nWindows workspace clippy leg now compiles browser-cef through the C1\nexample dependencies and rejects them.\n\nClaude-Session: https://claude.ai/code/session_01XwLTWGKnqhKDu4ym3qEobm",
          "timestamp": "2026-08-28T12:03:47Z",
          "url": "https://github.com/water-rs/waterui/commit/57535ef91f8aa0d740ea67420e967c3f0566b3f4"
        },
        "date": 1787943110798,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 7260301,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6635450,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 57684,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 43127,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788008299521,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 7703622,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6492823,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 70698,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 51532,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788088416216,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 8007529,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6845109,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 295073,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 127551,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788178979389,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 7019841,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6285969,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 70224,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 48580,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "id": "bf9545ce3621a1248944b209e137b26757dde472",
          "message": "docs: require GitHub issues and PRs targeting dev\n\nAgents must file each finding as a self-contained GitHub issue and land\nthe fix as a pull request against `dev`. Direct pushes to `dev` and\n`main` are no longer allowed. Issues must not be phased or sequenced\nslices of a larger plan.",
          "timestamp": "2026-08-29T04:01:35Z",
          "url": "https://github.com/water-rs/waterui/commit/bf9545ce3621a1248944b209e137b26757dde472"
        },
        "date": 1788260335204,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 7793728,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6680672,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 61039,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 43412,
            "unit": "us"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Lexo Liu",
            "username": "lexoliu",
            "email": "me@lexo.cool"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "ce31ee685c242b052555d89dd8b064c40f3eba95",
          "message": "Merge pull request #242 from water-rs/agent/hydrolysis-scene-engine-per-adapter-v2\n\nfix(hydrolysis): keep materialized views out of the address-keyed measure cache",
          "timestamp": "2026-09-02T09:22:39Z",
          "url": "https://github.com/water-rs/waterui/commit/ce31ee685c242b052555d89dd8b064c40f3eba95"
        },
        "date": 1788348304821,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame p95",
            "value": 6829201,
            "unit": "us"
          },
          {
            "name": "stress-example/stress_steady_redraw/steady-redraw frame mean",
            "value": 6009994,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame p95",
            "value": 81730,
            "unit": "us"
          },
          {
            "name": "list-example/list_wheel_scroll/wheel-scroll frame mean",
            "value": 50399,
            "unit": "us"
          }
        ]
      }
    ]
  }
}