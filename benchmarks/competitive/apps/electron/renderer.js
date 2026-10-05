// Competitive benchmark — Electron renderer, workloads W1–W4.
// Same constants and animation program as every other contestant.

const { ipcRenderer } = require('electron');

const params = new URLSearchParams(location.search);
const WORKLOAD = params.get('workload');
// The main process traps on missing/unrecognized args before the window
// loads; guard here too so a wrong page can never render silently.
if (!['W1', 'W2', 'W3', 'W4'].includes(WORKLOAD)) {
  throw new Error(
    `missing or unrecognized -bench-workload launch argument ` +
      `(got ${WORKLOAD ?? 'null'}); expected W1|W2|W3|W4`,
  );
}
const AUTO_DRIVE = params.get('drive') === 'auto';

const ROW_COLORS = [
  '#3B82F6', '#10B981', '#F59E0B', '#EF4444', '#8B5CF6', '#EC4899',
];

const PARAGRAPHS = [
  'The quick brown fox jumps over the lazy dog. 。🦊🐶 Packing my box with five dozen liquor jugs.',
  'WaterUI renders native widgets from a single Rust view tree. 。🌊 Fine-grained reactivity updates only the widgets that read the value.',
  'Almost all programming can be viewed as state management. ，。📚 Signals flow through the graph and wake the views that observe them.',
  'Sphinx of black quartz, judge my vow. のテキストもぜます。🗻 Typography is the visual component of the written word.',
  'How vexingly quick daft zebras jump! ，。🦓 The first principle is that you must not fool yourself.',
  'Bright vixens jump; dozy fowl quack. ，。🐦 Rendering pipelines measure progress in milliseconds per frame.',
  '。Benchmarks that are honest make optimisation honest. 📏',
  'Two driven jocks help fax my big quiz. ，。🌲 Lazily built lists keep memory flat while content grows without bound.',
  'The five boxing wizards jump quickly. ，。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.',
  'Jackdaws love my big sphinx of quartz. ，。🐦‍⬛ Measure, then optimise; never optimise on faith alone.',
];

const timestamp = i =>
  `${String(Math.floor(i / 60) % 24).padStart(2, '0')}:${String(i % 60).padStart(2, '0')}`;

function makeXorShift(seed) {
  let s = BigInt.asUintN(64, BigInt(seed));
  return () => {
    s ^= BigInt.asUintN(64, s << 13n);
    s ^= s >> 7n;
    s ^= BigInt.asUintN(64, s << 17n);
    s = BigInt.asUintN(64, s);
    return Number(s % 10000n) / 10000;
  };
}

// MARK: - W1 Hello

function renderHello(root) {
  root.innerHTML = `
    <div class="center">
      <div id="count">Count: 0</div>
      <button class="btn" id="increment-button" aria-label="Increment">Increment</button>
    </div>`;
  let n = 0;
  root.querySelector('#increment-button').addEventListener('click', () => {
    n += 1;
    root.querySelector('#count').textContent = `Count: ${n}`;
  });
}

// MARK: - Scroll self-drive (identical fling program on every platform)

// `auto` drive affordance: the runner's dev.bench.begin Darwin post is
// relayed by the main process over the 'bench-begin' IPC (a DOM button
// can't be tapped — the AX query is exactly what stalls past the test
// cap). ack/done go back over IPC for main to post as Darwin tokens.
function autoDriveListen(runProgram) {
  let running = false;
  ipcRenderer.on('bench-begin', () => {
    if (running) return;
    running = true;
    ipcRenderer.send('bench-ack');
    runProgram(() => {
      running = false;
      ipcRenderer.send('bench-done');
    });
  });
  ipcRenderer.send('bench-armed');
}

function markDone(root) {
  const el = document.createElement('div');
  el.id = 'bench-done';
  el.style.cssText = 'position:fixed;top:8px;right:8px;width:1px;height:1px';
  root.appendChild(el);
}

function driveFling(el, onDone, retries) {
  const max = el.scrollHeight - el.clientHeight;
  if (max <= 0) {
    // Layout may still be settling when `begin` lands right after load —
    // retry briefly rather than reporting a finished program.
    if ((retries || 0) < 50) {
      setTimeout(() => driveFling(el, onDone, (retries || 0) + 1), 300);
      return;
    }
    onDone();
    return;
  }
  const steps = [1, 2, 3, 4, 5, 6, 7, 8, 6, 3, 0].map(s => (max * s) / 8);
  let idx = 0;
  const runStep = () => {
    if (idx >= steps.length) {
      onDone();
      return;
    }
    const from = el.scrollTop;
    const to = steps[idx++];
    const t0 = performance.now();
    const tick = now => {
      const t = Math.min((now - t0) / 900, 1);
      const e = 1 - Math.pow(1 - t, 3); // easeOutCubic
      el.scrollTop = from + (to - from) * e;
      if (t < 1) requestAnimationFrame(tick);
      else setTimeout(runStep, 250);
    };
    requestAnimationFrame(tick);
  };
  setTimeout(runStep, 1000);
}

// MARK: - W2 Feed

function renderFeed(root) {
  root.innerHTML = '<div class="scroller" id="feed"></div>';
  const feed = root.querySelector('#feed');
  // Windowed list, the DOM analogue of cell reuse: a fixed-height spacer
  // gives real scroll geometry while only the visible rows (+overscan)
  // exist as elements. Row nodes are pooled and recycled like
  // UITableView's dequeueReusableCell.
  const ROW_H = 48, COUNT = 10000, OVERSCAN = 6;
  const inner = document.createElement('div');
  inner.className = 'vscroll';
  inner.style.height = `${COUNT * ROW_H}px`;
  feed.appendChild(inner);
  const pool = [];
  let lo = -1;
  const render = () => {
    const first = Math.max(0, Math.floor(feed.scrollTop / ROW_H) - OVERSCAN);
    const vis = Math.ceil(feed.clientHeight / ROW_H) + 2 * OVERSCAN;
    const last = Math.min(COUNT, first + vis);
    if (first === lo) return;
    lo = first;
    const need = last - first;
    while (pool.length < need) {
      const el = document.createElement('div');
      el.className = 'row';
      el.innerHTML = '<div class="avatar"></div><div class="row-lines">' +
        '<div class="row-title"></div><div class="row-sub"></div></div>' +
        '<div class="row-time"></div>';
      inner.appendChild(el);
      pool.push(el);
    }
    for (let k = 0; k < need; k++) {
      const i = first + k;
      const el = pool[k];
      el.style.display = '';
      el.style.transform = `translateY(${i * ROW_H}px)`;
      el.children[0].style.background = ROW_COLORS[i % 6];
      el.children[1].children[0].textContent = `Row title ${i}`;
      el.children[1].children[1].textContent =
        `Second line of subtitle for item ${i}`;
      el.children[2].textContent = timestamp(i);
    }
    for (let k = need; k < pool.length; k++) pool[k].style.display = 'none';
  };
  feed.addEventListener('scroll', render);
  render();
  if (AUTO_DRIVE) {
    autoDriveListen(onDone =>
      driveFling(feed, () => { markDone(root); onDone(); }),
    );
  }
}

// MARK: - W3 Motion

const FIELD_W = 720;
const FIELD_H = 440;
const RECT = 40;

function renderMotion(root) {
  root.innerHTML = '<div class="center"><div class="field" id="field"></div></div>';
  const field = root.querySelector('#field');
  for (let i = 0; i < 200; i++) {
    const init = makeXorShift(
      0xd1b54a32d192ed03n ^ BigInt(i) * 0x2545f4914f6cdd1dn,
    );
    init();
    const rng = makeXorShift(
      0x9e3779b97f4a7c15n ^ BigInt(i) * 0xbf58476d1ce4e5b9n,
    );
    const duration = 1200 + (i % 5) * 200;
    const el = document.createElement('div');
    el.className = 'rect';
    el.style.background = ROW_COLORS[i % 6];
    const pose = {
      x: init() * (FIELD_W - RECT),
      y: init() * (FIELD_H - RECT),
      rot: init() * 360,
      op: 0.3 + init() * 0.7,
    };
    const apply = p => {
      el.style.transform =
        `translate(${p.x}px, ${p.y}px) rotate(${p.rot}deg)`;
      el.style.opacity = p.op;
    };
    apply(pose);
    field.appendChild(el);
    const step = () => {
      const target = {
        x: rng() * (FIELD_W - RECT),
        y: rng() * (FIELD_H - RECT),
        rot: rng() * 360,
        op: 0.3 + rng() * 0.7,
      };
      const anim = el.animate(
        [
          {
            transform: `translate(${pose.x}px, ${pose.y}px) rotate(${pose.rot}deg)`,
            opacity: pose.op,
          },
          {
            transform: `translate(${target.x}px, ${target.y}px) rotate(${target.rot}deg)`,
            opacity: target.op,
          },
        ],
        { duration, easing: 'ease-in-out', fill: 'forwards' },
      );
      anim.onfinish = () => {
        apply(target);
        Object.assign(pose, target);
        step();
      };
    };
    setTimeout(step, duration);
  }
}

// MARK: - W4 Text

function renderText(root) {
  root.innerHTML = '<div class="scroller" id="sc"></div>';
  const sc = root.querySelector('#sc');
  const html = [];
  for (let i = 0; i < 50; i++) {
    html.push(`<div class="paragraph">${PARAGRAPHS[i % PARAGRAPHS.length]}</div>`);
  }
  sc.innerHTML = html.join('');
  if (AUTO_DRIVE) {
    autoDriveListen(onDone =>
      driveFling(sc, () => { markDone(root); onDone(); }),
    );
  }
}

// MARK: - Root

const root = document.getElementById('root');
// Workload identity for the accessibility tree — as a data attribute and
// aria-label, NOT the id: renaming the element would unmatch the
// `#root{height:100%}` rule and unbound the scrollers (scrollHeight ==
// clientHeight → driveFling measures max=0 and finishes instantly).
root.setAttribute('data-workload', WORKLOAD);
root.setAttribute('role', 'main');
root.setAttribute('aria-label', `workload ${WORKLOAD}`);
switch (WORKLOAD) {
  case 'W2': renderFeed(root); break;
  case 'W3': renderMotion(root); break;
  case 'W4': renderText(root); break;
  default: renderHello(root);
}
