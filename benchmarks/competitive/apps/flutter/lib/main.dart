// Competitive benchmark — Flutter contestant, workloads W1–W6, per
// benchmarks/competitive/WORKLOADS.md. Workload selection:
// `-bench-workload W1|W2|W3|W4|W5|W6` (and `-bench-step N` for W5/W6),
// read through NSUserDefaults' NSArgumentDomain via the bench/config
// channel. Scrolling is driven from outside the app by OS-level input —
// the app never scrolls itself. On Apple the W5/W6 ladder waits for the
// runner's `dev.bench.begin` Darwin post inside the measure block (polled
// through bench/config; AX cannot carry the signal because a timed-out
// AX query fails the test) and posts `dev.bench.done` at the end.

import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

const _configChannel = MethodChannel('bench/config');

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final workload = (await _readConfig('workload'))?.toUpperCase();
  if (!const ['W1', 'W2', 'W3', 'W4', 'W5', 'W6'].contains(workload)) {
    throw StateError(
        'missing or unrecognized -bench-workload launch argument '
        '(got ${workload ?? 'null'}); expected W1..=W6');
  }
  final stepStr = await _readConfig('step');
  final pinnedStep = stepStr == null ? null : int.tryParse(stepStr);
  if (stepStr != null && pinnedStep == null) {
    throw StateError('malformed -bench-step value $stepStr; expected integer');
  }
  final selfPaced = Platform.isIOS || Platform.isMacOS;
  if (const ['W5', 'W6'].contains(workload)) {
    final ladder = workload == 'W5'
        ? MotionCapacityPage.steps
        : FeedCapacityPage.steps;
    if (pinnedStep != null && !ladder.contains(pinnedStep)) {
      throw StateError(
          'unrecognized -bench-step value $pinnedStep for $workload; '
          'expected one of $ladder');
    }
    if (pinnedStep == null && !selfPaced) {
      throw StateError(
          '$workload requires -bench-step (or BENCH_STEP); this leg '
          'measures one ladder step per launch');
    }
  }
  runApp(BenchApp(workload: workload!, pinnedStep: pinnedStep));
}

/// Reads a benchmark config value. On Apple targets the platform channel
/// serves it from NSUserDefaults' NSArgumentDomain (launch arguments);
/// on Android the override MainActivity serves the same channel from
/// intent extras; desktop legs pass `BENCH_<NAME>` in the environment.
/// A missing or unrecognized value traps — never a silent fallback.
Future<String?> _readConfig(String name) async {
  try {
    final v = await _configChannel.invokeMethod<String>(name);
    if (v != null && v.isNotEmpty) return v;
  } catch (_) {}
  try {
    final v = Platform.environment['BENCH_${name.toUpperCase()}'];
    if (v != null && v.isNotEmpty) return v;
  } catch (_) {}
  return null;
}

class BenchApp extends StatelessWidget {
  const BenchApp({super.key, required this.workload, this.pinnedStep});
  final String workload;

  /// The per-launch capacity protocol: one ladder step per app start on
  /// Android/desktop, arriving as the `step` config value. Null on Apple =
  /// the self-paced in-app ladder driven by the bench handshake.
  final int? pinnedStep;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Bench',
      // The runner asserts this accessibility identifier after launch.
      home: Semantics(
        identifier: 'bench-workload-$workload',
        child: switch (workload) {
          'W2' => const FeedPage(),
          'W3' => const MotionPage(),
          'W4' => const TextPage(),
          'W5' => MotionCapacityPage(pinnedStep: pinnedStep),
          'W6' => FeedCapacityPage(pinnedStep: pinnedStep),
          _ => const HelloPage(),
        },
      ),
    );
  }
}

void _ready() {
  // BENCH_READY on stdout is the runner's launch-timing signal.
  // ignore: avoid_print
  SchedulerBinding.instance.addPostFrameCallback((_) => print('BENCH_READY'));
}

// MARK: - Shared constants (identical across contestants)

const rowColors = [
  Color(0xFF3B82F6), Color(0xFF10B981), Color(0xFFF59E0B),
  Color(0xFFEF4444), Color(0xFF8B5CF6), Color(0xFFEC4899),
];

String timestamp(int i) =>
    '${((i ~/ 60) % 24).toString().padLeft(2, '0')}:${(i % 60).toString().padLeft(2, '0')}';

const paragraphs = [
  'The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸跳過懶惰的狗。🦊🐶 Packing my box with five dozen liquor jugs.',
  'WaterUI renders native widgets from a single Rust view tree. 水のインターフェースはネイティブウィジェットを描画する。🌊',
  'Almost all programming can be viewed as state management. 几乎所有的编程都可以视为状态管理。📚 Signals flow through the graph.',
  'Sphinx of black quartz, judge my vow. 黒い水晶のスフィンクス、私の誓いを裁け。🗻 Typography is the visual component of the written word.',
  'How vexingly quick daft zebras jump! 빠른 얼룩말이 얼마나 성가시게 뛰는가! 🦓 The first principle is that you must not fool yourself.',
  'Bright vixens jump; dozy fowl quack. 밝은 여우가 뛰고 졸린 새가 꽥꽥 운다. 🐦 Rendering pipelines measure progress in milliseconds per frame.',
  'ベンチマークが正直であれば最適化も正直になる。Benchmarks that are honest make optimisation honest. 📏',
  'Two driven jocks help fax my big quiz. 두 명의 조키가 내 큰 퀴즈를 팩스로 보내는 것을 돕는다. 🌲 Lazily built lists keep memory flat.',
  'The five boxing wizards jump quickly. 五個拳擊巫師跳得很快。🧙 Every frame has a budget of 8.33 milliseconds at 120 Hz.',
  'Jackdaws love my big sphinx of quartz. 寒鸦喜欢我巨大的石英斯芬克斯。🐦‍⬛ Measure, then optimise; never optimise on faith alone.',
];

/// Deterministic PRNG so every contestant animates the same sequence —
/// xorshift64 on unsigned 64-bit state (WORKLOADS.md): `>>>` is the
/// logical shift (arithmetic `>>` sign-extends and breaks the stream) and
/// `state` is a signed int64 holding u64 bits, so `u64 % 10000` is computed
/// in two 32-bit halves (2^32 mod 10000 = 7296).
class XorShift {
  int state;
  XorShift(int seed) : state = seed & 0xFFFFFFFFFFFFFFFF;
  double next() {
    state ^= (state << 13) & 0xFFFFFFFFFFFFFFFF;
    state ^= state >>> 7;
    state ^= (state << 17) & 0xFFFFFFFFFFFFFFFF;
    final hi = state >>> 32;
    final lo = state & 0xFFFFFFFF;
    return ((hi % 10000) * 7296 + (lo % 10000)) % 10000 / 10000.0;
  }
}

// MARK: - W1 Hello

class HelloPage extends StatefulWidget {
  const HelloPage({super.key});
  @override
  State<HelloPage> createState() => _HelloPageState();
}

class _HelloPageState extends State<HelloPage> {
  int counter = 0;
  @override
  void initState() {
    super.initState();
    _ready();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('Count: $counter'),
            const SizedBox(height: 16),
            Semantics(
              identifier: 'increment-button',
              button: true,
              child: ElevatedButton(
                onPressed: () => setState(() => counter++),
                child: const Text('Increment'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

// MARK: - Apple capacity-ladder handshake (measurement pacing, not a scroll drive)

/// Waits for the runner's `dev.bench.begin` post, forwarded by the native
/// side of bench/config. The ladder must not start before the measure
/// block signals it.
Future<void> _awaitBegin() async {
  while (true) {
    try {
      if (await _configChannel.invokeMethod<bool>('beginObserved') == true) {
        return;
      }
    } catch (_) {}
    await Future.delayed(const Duration(milliseconds: 50));
  }
}

/// Posts `dev.bench.done` back to the runner when the program finishes.
Future<void> _postDone() async {
  try {
    await _configChannel.invokeMethod('postDone');
  } catch (_) {}
}

/// Logs `step k n=<param> t=<unix>` to tmp/bench-steps.log and posts
/// `dev.bench.step` — the runner slices its xctrace recording by the
/// logged times. Identical on every contestant.
Future<void> _logStep(int step, int n) async {
  try {
    await _configChannel
        .invokeMethod('logStep', {'step': step, 'n': n});
  } catch (_) {}
}

// MARK: - W2 Feed

/// Row of the W2 feed — geometry, text and palette per WORKLOADS.md.
Widget feedRow(int i, [List<Widget> extra = const []]) {
  return Padding(
    padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
    child: Row(
      children: [
        Container(
          width: 40, height: 40,
          decoration: BoxDecoration(
            color: rowColors[i % 6],
            shape: BoxShape.circle,
          ),
        ),
        const SizedBox(width: 12),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text('Row title $i',
                  style: const TextStyle(fontSize: 16)),
              Text('Second line of subtitle for item $i',
                  style: TextStyle(
                      fontSize: 13, color: Colors.grey.shade600)),
            ],
          ),
        ),
        ...extra,
        Text(timestamp(i),
            style: TextStyle(fontSize: 13, color: Colors.grey.shade600)),
      ],
    ),
  );
}

class FeedPage extends StatefulWidget {
  const FeedPage({super.key});
  @override
  State<FeedPage> createState() => _FeedPageState();
}

class _FeedPageState extends State<FeedPage> {
  @override
  void initState() {
    super.initState();
    _ready();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: ListView.builder(
        itemCount: 10000,
        itemBuilder: (context, i) => feedRow(i),
      ),
    );
  }
}

// MARK: - W3 Motion

const _fieldW = 720.0;
const _fieldH = 440.0;
const _rectSize = 40.0;

class MotionPage extends StatelessWidget {
  const MotionPage({super.key});
  @override
  Widget build(BuildContext context) {
    _ready();
    return Scaffold(
      body: Center(
        child: SizedBox(
          width: _fieldW,
          height: _fieldH,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              for (var i = 0; i < 200; i++) MotionRect(index: i),
            ],
          ),
        ),
      ),
    );
  }
}

class MotionRect extends StatefulWidget {
  const MotionRect({super.key, required this.index});
  final int index;
  @override
  State<MotionRect> createState() => _MotionRectState();
}

class _MotionRectState extends State<MotionRect> {
  late double x, y, rot, op;
  late final XorShift _rng;
  late final Duration _duration;
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    final i = widget.index;
    var initRng = XorShift(0xD1B54A32D192ED03 ^ i * 0x2545F4914F6CDD1D);
    x = initRng.next() * (_fieldW - _rectSize);
    y = initRng.next() * (_fieldH - _rectSize);
    rot = initRng.next() * 360;
    op = 0.3 + initRng.next() * 0.7;
    _rng = XorShift(0x9E3779B97F4A7C15 ^ i * 0xBF58476D1CE4E5B9);
    _duration = Duration(milliseconds: 1200 + (i % 5) * 200);
    // Retarget at t=0, then each time this rect's animation completes.
    _step(init: true);
    _timer = Timer.periodic(_duration, (_) => _step());
  }

  /// Pulls the next drive-stream target. In initState the rect's fields are
  /// assigned directly (the first build animates init pose → first target);
  /// after that a setState retargets.
  void _step({bool init = false}) {
    final nx = _rng.next() * (_fieldW - _rectSize);
    final ny = _rng.next() * (_fieldH - _rectSize);
    final nrot = _rng.next() * 360;
    final nop = 0.3 + _rng.next() * 0.7;
    if (init) {
      x = nx;
      y = ny;
      rot = nrot;
      op = nop;
    } else {
      setState(() {
        x = nx;
        y = ny;
        rot = nrot;
        op = nop;
      });
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedPositioned(
      duration: _duration,
      curve: Curves.easeInOut,
      left: x,
      top: y,
      child: AnimatedOpacity(
        duration: _duration,
        curve: Curves.easeInOut,
        opacity: op,
        child: TweenAnimationBuilder<double>(
          tween: Tween(begin: 0, end: rot),
          duration: _duration,
          curve: Curves.easeInOut,
          builder: (context, r, child) =>
              Transform.rotate(angle: r * 3.141592653589793 / 180, child: child),
          child: Container(
            width: _rectSize,
            height: _rectSize,
            decoration: BoxDecoration(
              color: rowColors[widget.index % 6],
              borderRadius: BorderRadius.circular(10),
            ),
          ),
        ),
      ),
    );
  }
}

// MARK: - W4 Text

/// W4: all 50 paragraphs laid out eagerly — layout cost is part of the
/// measurement, so the list is not lazy.
class TextPage extends StatefulWidget {
  const TextPage({super.key});
  @override
  State<TextPage> createState() => _TextPageState();
}

class _TextPageState extends State<TextPage> {
  @override
  void initState() {
    super.initState();
    _ready();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (var i = 0; i < 50; i++)
              Padding(
                padding: const EdgeInsets.symmetric(
                    horizontal: 16, vertical: 10),
                child: Text(paragraphs[i % paragraphs.length],
                    style: const TextStyle(fontSize: 16)),
              ),
          ],
        ),
      ),
    );
  }
}

// MARK: - W5 Motion capacity

/// W3's scene with the rect count doubled per step (200…25600): after
/// `dev.bench.begin`, each step logs its boundary, holds 5 s, advances;
/// `dev.bench.done` ends the program.
class MotionCapacityPage extends StatefulWidget {
  const MotionCapacityPage({super.key, this.pinnedStep});
  final int? pinnedStep;
  static const steps = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];
  @override
  State<MotionCapacityPage> createState() => _MotionCapacityPageState();
}

class _MotionCapacityPageState extends State<MotionCapacityPage> {
  late int _count = widget.pinnedStep ?? MotionCapacityPage.steps.first;

  @override
  void initState() {
    super.initState();
    _ready();
    // Apple leg: the in-app ladder is driven by the begin/done handshake;
    // other legs pin one step per launch (main() already traps if absent).
    if (widget.pinnedStep == null) {
      () async {
        while (true) {
          await _awaitBegin();
          for (var i = 0; i < MotionCapacityPage.steps.length; i++) {
            setState(() => _count = MotionCapacityPage.steps[i]);
            await _logStep(i, _count);
            await Future.delayed(const Duration(seconds: 5));
          }
          await _postDone();
          await _configChannel.invokeMethod('discardBegins');
        }
      }();
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: SizedBox(
          width: _fieldW,
          height: _fieldH,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              for (var i = 0; i < _count; i++) MotionRect(index: i),
            ],
          ),
        ),
      ),
    );
  }
}

// MARK: - W6 Feed capacity

/// W2's fling program over rows whose nested text+shape child count
/// doubles per step (1…64); two full sweeps inside each 5 s hold.
/// W2's feed rows whose nested text+shape cell count doubles per step
/// (1…64); the runner's OS-level fling program runs during each hold.
class FeedCapacityPage extends StatefulWidget {
  const FeedCapacityPage({super.key, this.pinnedStep});
  final int? pinnedStep;
  static const steps = [1, 2, 4, 8, 16, 32, 64];
  @override
  State<FeedCapacityPage> createState() => _FeedCapacityPageState();
}

class _FeedCapacityPageState extends State<FeedCapacityPage> {
  late int _complexity = widget.pinnedStep ?? FeedCapacityPage.steps.first;

  @override
  void initState() {
    super.initState();
    _ready();
    // Apple leg: the in-app ladder is driven by the begin/done handshake;
    // other legs pin one step per launch (main() already traps if absent).
    if (widget.pinnedStep == null) {
      () async {
        while (true) {
          await _awaitBegin();
          for (var i = 0; i < FeedCapacityPage.steps.length; i++) {
            setState(() => _complexity = FeedCapacityPage.steps[i]);
            await _logStep(i, _complexity);
            // settle 1 s + hold 4 s; the runner drives the flings.
            await Future.delayed(const Duration(seconds: 5));
          }
          await _postDone();
          await _configChannel.invokeMethod('discardBegins');
        }
      }();
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: ListView.builder(
        itemCount: 10000,
        itemBuilder: (context, i) => feedRow(i, [
          for (var j = 0; j < _complexity; j++)
            Column(children: [
              Container(
                width: 14, height: 14,
                decoration: BoxDecoration(
                  color: rowColors[(i + j) % 6],
                  borderRadius: BorderRadius.circular(4),
                ),
              ),
              Text('c$j',
                  style: TextStyle(
                      fontSize: 12, color: Colors.grey.shade600)),
            ]),
        ]),
      ),
    );
  }
}
