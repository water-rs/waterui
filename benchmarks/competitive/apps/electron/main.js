// Competitive benchmark — shared Electron contestant, workloads W1–W5.
// Workload selection accepts both channels a leg can deliver:
// `-bench-workload W1|W2|W3|W4|W5` on argv (Apple legs) or `BENCH_WORKLOAD`
// in the environment (Linux, Windows). Missing or unrecognized workload
// traps — never silently measure W1. Scrolling is the runner's OS-level
// input; the app never scrolls itself. BENCH_READY on stdout marks the
// first committed frame.

const { app, BrowserWindow } = require('electron');
const { execFile, execFileSync } = require('child_process');

// A persistent user-data-dir serves index.html/renderer.js from the disk
// cache — a stale renderer once ran after a rebuild. Fresh userData per
// launch also makes every measured cell a cold start.
{
  const os = require('os');
  const fs = require('fs');
  const path = require('path');
  app.setPath(
    'userData',
    fs.mkdtempSync(path.join(os.tmpdir(), 'electron-bench-')),
  );
}
function benchArg(name) {
  const i = process.argv.indexOf(`-bench-${name}`);
  if (i >= 0 && i + 1 < process.argv.length) return process.argv[i + 1];
  return process.env[`BENCH_${name.toUpperCase()}`] || null;
}

const workload = benchArg('workload')?.toUpperCase();
if (!['W1', 'W2', 'W3', 'W4', 'W5'].includes(workload)) {
  console.error(
    `missing or unrecognized workload ` +
      `(got ${workload === null ? 'nil' : workload}); expected W1|W2|W3|W4|W5`,
  );
  process.exit(1);
}
// W5 is the canonical capacity ladder (../WORKLOADS.md): one ladder step
// per launch, pinned by -bench-step / BENCH_STEP — a missing, malformed
// or off-ladder step traps, never a default.
const W5_STEPS = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];
let step = null;
if (workload === 'W5') {
  const raw = benchArg('step');
  step = raw === null ? null : Number(raw);
  if (step === null || !W5_STEPS.includes(step)) {
    console.error(
      `W5 requires -bench-step (or BENCH_STEP) naming a ladder member ` +
        `(got ${raw === null ? 'nil' : raw}); expected one of ${W5_STEPS}`,
    );
    process.exit(1);
  }
}
// BENCH_GPUINFO: the Windows leg's actual-renderer evidence — the GPU
// info Electron itself selected for this process, emitted once ready so
// the runner can bind it to the owned process tree.
app.whenReady().then(async () => {
  try {
    const info = await app.getGPUInfo('complete');
    process.stdout.write(`BENCH_GPUINFO ${JSON.stringify(info)}\n`);
  } catch (e) {
    process.stdout.write(`BENCH_GPUINFO_ERROR ${e}\n`);
  }
});

// Apple legs wait for `dev.bench.ready.<bundle-id>.<W>` to confirm the
// workload argument arrived — a deep AX query on the 10k-row feed stalls
// for minutes, so a Darwin notification carries the assertion. Node has
// no notify binding; `/usr/bin/notifyutil -p` posts the same token.
const notify =
  process.platform === 'darwin'
    ? name => execFile('/usr/bin/notifyutil', ['-p', name], () => {})
    : () => {};
if (process.platform === 'darwin') {
  const path = require('path');
  let bundleID = 'dev.bench.electron';
  try {
    const plist = path.join(__dirname, '..', '..', 'Info.plist');
    bundleID = execFileSync(
      '/usr/libexec/PlistBuddy',
      ['-c', 'Print:CFBundleIdentifier', plist],
    ).toString().trim() || bundleID;
  } catch { /* keep the packaged bundle id */ }
  execFileSync('/usr/bin/notifyutil', [
    '-p', `dev.bench.ready.${bundleID}.${workload}`,
  ]);
}

let win = null;

app.whenReady().then(() => {
  win = new BrowserWindow({
    // The 1280x800 window every contestant gets (WORKLOADS.md).
    width: 1280,
    height: 800,
    // hidden until the first frame is committed so the visible first
    // present is the measured frame — not a white flash
    show: false,
    webPreferences: {
      sandbox: false,
      // renderer.js only paints the workload; Node in the
      // app only ever loads its bundled index.html, so direct Node in
      // the renderer is safe here.
      nodeIntegration: true,
      contextIsolation: false,
    },
  });
  win.removeMenu();
  // did-frame-finish-load fires when the first frame is committed —
  // the closest Electron equivalent of "first frame on screen".
  win.webContents.once('did-frame-finish-load', () => {
    win.show();
    process.stdout.write('BENCH_READY\n');
  });
  win.loadFile('index.html', {
    query: { workload, step },
  });
});

app.on('window-all-closed', () => app.quit());
