// Competitive benchmark — shared Electron contestant, workloads w1–w6
// (exact lowercase ids). Selection accepts both channels a leg can
// deliver: `-bench-workload w1|w2|w3|w4|w5|w6` on argv (Apple legs) or
// BENCH_WORKLOAD in the environment (Linux, Windows). Missing or
// unrecognized workload traps — never silently measure w1. Scrolling is
// the runner's OS-level input; the app never scrolls itself. BENCH_READY
// on stdout marks the first committed frame. The host driver ends
// every cell on its own schedule — the app posts readiness only,
// never completion.

const { app, BrowserWindow } = require('electron');
const { execFile, execFileSync } = require('child_process');

// A persistent user-data-dir serves index.html/renderer.js from the disk
// cache — a stale renderer once ran after a rebuild. Fresh userData per
// launch also makes every measured cell a cold start.
let benchUserData;
{
  const os = require('os');
  const fs = require('fs');
  const path = require('path');
  benchUserData = fs.mkdtempSync(path.join(os.tmpdir(), 'electron-bench-'));
  app.setPath('userData', benchUserData);
  // The per-launch userData dir is thrown away on exit — no leak.
  app.on('quit', () => {
    try {
      fs.rmSync(benchUserData, { recursive: true, force: true });
    } catch (e) {
      process.stderr.write(`userData cleanup failed: ${e}\n`);
    }
  });
}
function benchArg(name) {
  const i = process.argv.indexOf(`-bench-${name}`);
  if (i >= 0 && i + 1 < process.argv.length) return process.argv[i + 1];
  return process.env[`BENCH_${name.toUpperCase()}`] || null;
}

const workload = benchArg('workload');
if (!['w1', 'w2', 'w3', 'w4', 'w5', 'w6'].includes(workload)) {
  console.error(
    `missing or unrecognized workload ` +
      `(got ${workload === null ? 'nil' : workload}); expected w1|w2|w3|w4|w5|w6`,
  );
  process.exit(1);
}
// Capacity ladders (../WORKLOADS.md): one ladder step per launch, pinned
// by -bench-step / BENCH_STEP — a missing, malformed or off-ladder step
// traps, never a default.
const STEPS = {
  w5: [200, 400, 800, 1600, 3200, 6400, 12800, 25600],
  w6: [1, 2, 4, 8, 16, 32, 64],
};
let step = null;
if (workload === 'w5' || workload === 'w6') {
  const raw = benchArg('step');
  step = raw === null ? null : Number(raw);
  if (step === null || !STEPS[workload].includes(step)) {
    console.error(
      `${workload} requires -bench-step (or BENCH_STEP) naming a ` +
        `ladder member (got ${raw === null ? 'nil' : raw}); ` +
        `expected one of ${STEPS[workload]}`,
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

// Apple legs wait for `dev.bench.ready.<bundle-id>.<w>` to confirm the
// workload argument arrived — a deep AX query on the 10k-row feed stalls
// for minutes, so a Darwin notification carries the assertion. Node has
// no notify binding; `/usr/bin/notifyutil -p` posts the same token.
const notifyPost =
  process.platform === 'darwin'
    ? name => execFile('/usr/bin/notifyutil', ['-p', name], () => {})
    : () => {};
if (process.platform === 'darwin') {
  // The ready post carries the packaged bundle id — there is no default:
  // if the plist cannot be read the launch fails, never a wrong token.
  const path = require('path');
  let bundleID;
  try {
    const plist = path.join(__dirname, '..', '..', 'Info.plist');
    bundleID = execFileSync(
      '/usr/libexec/PlistBuddy',
      ['-c', 'Print:CFBundleIdentifier', plist],
    ).toString().trim();
  } catch (e) {
    console.error(`cannot read CFBundleIdentifier from Info.plist: ${e}`);
    process.exit(1);
  }
  if (!bundleID) {
    console.error('Info.plist has an empty CFBundleIdentifier');
    process.exit(1);
  }
  notifyPost(`dev.bench.ready.${bundleID}.${workload}`);
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
  // ready-to-show fires when the initial frame is rendered and the
  // window can be presented — the readiness signal, not a guessed paint.
  win.once('ready-to-show', () => {
    win.show();
    process.stdout.write('BENCH_READY\n');
  });
  win.loadFile('index.html', {
    query: { workload, step },
  });
});

app.on('window-all-closed', () => app.quit());
