// Competitive benchmark — shared Electron contestant, workloads W1–W4.
// Workload selection accepts both channels a leg can deliver:
// `-bench-workload W1|W2|W3|W4 -bench-drive swipe|auto` on argv (Apple legs)
// or `BENCH_WORKLOAD`/`BENCH_DRIVE` in the environment (Linux, Windows).
// Missing or unrecognized workload traps — never silently measure W1.
// BENCH_READY on stdout marks the first committed frame.

const { app, BrowserWindow, ipcMain } = require('electron');
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

const workload = benchArg('workload');
if (!['W1', 'W2', 'W3', 'W4'].includes(workload)) {
  console.error(
    `missing or unrecognized workload ` +
      `(got ${workload === null ? 'nil' : workload}); expected W1|W2|W3|W4`,
  );
  process.exit(1);
}
const drive = benchArg('drive') || 'swipe';
if (!['swipe', 'auto'].includes(drive)) {
  console.error(`unrecognized drive value ${drive}; expected swipe|auto`);
  process.exit(1);
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

// `auto` drive (Apple legs): the runner posts dev.bench.begin inside its
// measure block. A `notifyutil -1` child exits on the first posting — the
// only Darwin-notify listener Node can build. The renderer arms the
// channel once loaded and acks once the program starts (which stops the
// runner's begin reposts); dev.bench.done goes back the same way. An
// AX-tapped DOM button can't be used here: evaluating any AX query on
// the 10k-row feed is exactly what stalls past the test cap.
let win = null;
if (drive === 'auto' && process.platform === 'darwin') {
  app.whenReady().then(() => {
    // Every `dev.bench.begin` post is forwarded while armed — XCTest runs
    // the measure block more than once (an unmeasured warm-up pass plus
    // the measured pass), and each pass needs its own begin/ack/done
    // cycle. The renderer's `running` flag dedupes posts consumed while a
    // program is already in flight.
    let armed = false;
    const listenBegin = () =>
      execFile('/usr/bin/notifyutil', ['-1', 'dev.bench.begin'], () => {
        if (armed) win.webContents.send('bench-begin');
        listenBegin();
      });
    listenBegin();
    ipcMain.on('bench-armed', () => {
      armed = true;
    });
    ipcMain.on('bench-ack', () => notify('dev.bench.ack'));
    ipcMain.on('bench-done', () => notify('dev.bench.done'));
  });
}

app.whenReady().then(() => {
  win = new BrowserWindow({
    width: 960,
    height: 640,
    webPreferences: {
      sandbox: false,
      // renderer.js uses ipcRenderer for the auto-drive handshake; the
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
    process.stdout.write('BENCH_READY\n');
  });
  win.loadFile('index.html', {
    query: { workload, drive },
  });
});

app.on('window-all-closed', () => app.quit());
