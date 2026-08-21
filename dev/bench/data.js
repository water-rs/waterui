window.BENCHMARK_DATA = {
  "lastUpdate": 1787307651200,
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
      }
    ]
  }
}