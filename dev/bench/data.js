window.BENCHMARK_DATA = {
  "lastUpdate": 1787252670239,
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
      }
    ]
  }
}