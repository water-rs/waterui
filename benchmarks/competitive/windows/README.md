# Competitive benchmark — Windows

Windows leg of the competitive benchmark defined in
[water-rs/waterui#1262](https://github.com/water-rs/waterui/issues/1262):
WaterUI (hydrolysis) vs Flutter, Electron, WinUI 3, measured identically on
workloads W1–W4.

## Layout

| path | what |
|---|---|
| `manifest.toml` | every toolchain/framework pin; `run.py` reads it |
| `run.py` | single entry point (`uv run`) — builds contestants, launches each workload, samples memory, captures per-frame submission timestamps from one ETW session per run (DXGI + Kernel-Process, the stream PresentMon consumes) in file and real-time mode at once — the real-time stream reports the first owned present the drive is scheduled on, and the .etl's first owned present must be that identical event. Precondition: the foreground lock time-out (`HKCU\Control Panel\Desktop\ForegroundLockTimeout`) is 0 for the measuring user — checked before any cell |
| `report.py` | renders a results JSON into markdown tables |
| `apps/` | one idiomatic app per framework, each implementing W1–W4 selected by `BENCH_WORKLOAD`; `apps/waterui` is a `water create` project |

## Method

Every metric is the median of ≥5 runs with min/max and all samples kept.
Frame timing uses one declared source per contestant (`[frame_source]` in
`manifest.toml`): DXGI Present Start events for swapchain presenters
(WaterUI, Flutter, Electron) and DXGI event 144 for WinUI 3's
composition-path presents, which PresentMon 2.5.1 cannot time. The same
source anchors the drive and is parsed from the trace; no other stream is
consulted. `dropped_pct` counts frame intervals over 1.5× the 60 Hz
budget.

**Publishable frame-time and memory numbers require a hardware GPU host.**
The runner records the GPU adapters (`Win32_VideoController`) in
`results[].machine.gpu`; when every adapter is a software rasterizer (WARP /
Microsoft Basic Render Driver / SwiftShader-class) it refuses — exit 2 — to
emit frame-time or memory results. Package-size figures are unaffected by
the GPU.

Availability is not attribution: every contestant row must carry per-run
evidence of the adapter it actually rendered on, gathered from the owned
process tree only:

- **WaterUI** — hydrolysis's own `selected wgpu adapter` log line
  (`RUST_LOG=hydrolysis::gpu=info`) from the measured process;
- **Electron** — `app.getGPUInfo('complete')` reported by the measured
  process (`BENCH_GPUINFO` in its captured output);
- **Flutter / WinUI 3** — the owned pid set's
  `\GPU Engine(*)\Utilization Percentage` perf counters, whose instance
  names carry the adapter LUID of the GPU work, resolved to adapter
  identity through `IDXGIFactory1::EnumAdapters1`/`IDXGIAdapter::GetDesc`
  via comtypes.

A rep with missing or ambiguous evidence (no log line, no GPU-engine
attribution, an unmapped LUID) is recorded as a failed attempt and never
counted; a proven software adapter (Cpu-type, WARP/Basic Render,
SwiftShader, llvmpipe-class names) refuses the measurement with exit 2 —
under `--development` as well. There is no software-GPU measurement
route in this runner.

## Run

```powershell
uv run run.py                    # build + measure, 5 reps
uv run run.py --skip-build
uv run run.py --self-test        # CPU-only control/evidence checks
uv run run.py --development      # label output development-only (no GPU measurement path changes)
uv run report.py                 # newest results/*.json -> report.md
```

Prereqs: Windows host, `uv`, the pinned toolchain from `manifest.toml`
(PresentMon for reference, dotnet SDK, node, flutter; the DXC runtime
comes from `water package` and is checked against the `dxc` pin). The `water`
CLI is provisioned from this checkout (`cli/` is a workspace member —
`cargo install --locked --path cli` into the suite-shared cache) and the
checkout HEAD sha is recorded as the framework+CLI+backend identity. Python deps are pinned
in `pyproject.toml`/`uv.lock`: `psutil`, plus Windows-only `pywin32`
(Job Object / CreateProcess / pipes) and `comtypes` (DXGI COM) —
maintained bindings, no hand-rolled ctypes ABI. Process ownership is a
Job Object: the root launches suspended via `CreateProcess`
(`win32con.CREATE_SUSPENDED`), is assigned to the job before
`ResumeThread`, and the job's `ProcessIdList` is the sole
memory/termination attribution set.

## Limitations

- Committed results must come from hardware; a software-only host is
  always refused.
- On a VM with an indirect display (e.g. IddSampleDriver),
  display-finalisation timestamps are unavailable; `dropped_pct` is the
  frame-interval proxy described in `manifest.toml`.
- Electron requires no extra flags beyond what `run.py` passes.
