window.BENCHMARK_DATA = {
  "lastUpdate": 1787468694116,
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
      }
    ]
  }
}