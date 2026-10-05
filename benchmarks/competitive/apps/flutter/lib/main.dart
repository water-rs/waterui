// Competitive benchmark — Flutter contestant, workloads W1–W6.
// Workload selection: `-bench-workload W1|W2|W3|W4|W5|W6 -bench-drive swipe|auto`,
// read through NSUserDefaults' NSArgumentDomain via the bench/config channel
// (the native side traps on missing/unrecognized values before Dart runs).
// `auto` runs the built-in fling program only after the runner posts the
// `dev.bench.begin` Darwin notification inside its measure block (polled
// through the bench/config channel; AX cannot carry the signal because a
// timed-out AX query fails the test), and posts `dev.bench.done` at the end.

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
  final drive = await _readConfig('drive') ?? 'swipe';
  final stepStr = await _readConfig('step');
  final pinnedStep = stepStr == null ? null : int.tryParse(stepStr);
  if (drive != 'swipe' && drive != 'auto') {
    throw StateError(
        'unrecognized -bench-drive value $drive; expected swipe|auto');
  }
  runApp(BenchApp(
      workload: workload!, autoDrive: drive == 'auto', pinnedStep: pinnedStep));
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
  const BenchApp(
      {super.key,
      required this.workload,
      required this.autoDrive,
      this.pinnedStep});
  final String workload;
  final bool autoDrive;

  /// The Android leg's capacity protocol: one launch per ladder step, the
  /// step arriving as the `step` config value. Null = the auto ladder.
  final int? pinnedStep;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Bench',
      // The runner asserts this accessibility identifier after launch.
      home: Semantics(
        identifier: 'bench-workload-$workload',
        child: switch (workload) {
          'W2' => FeedPage(autoDrive: autoDrive),
          'W3' => const MotionPage(),
          'W4' => TextPage(autoDrive: autoDrive),
          'W5' => MotionCapacityPage(
              autoDrive: autoDrive, pinnedStep: pinnedStep),
          'W6' => FeedCapacityPage(
              autoDrive: autoDrive, pinnedStep: pinnedStep),
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

/// Deterministic PRNG so every contestant animates the same sequence.
class XorShift {
  int state;
  XorShift(int seed) : state = seed & 0xFFFFFFFFFFFFFFFF;
  double next() {
    state ^= (state << 13) & 0xFFFFFFFFFFFFFFFF;
    state ^= state >> 7;
    state ^= (state << 17) & 0xFFFFFFFFFFFFFFFF;
    return (state % 10000) / 10000.0;
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

// MARK: - Scroll self-drive (identical fling program on every platform)

/// Waits for the runner's `dev.bench.begin` post, forwarded by the native
/// side of bench/config. The program must not start before the measure
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

/// Runs the shared fling program on each `dev.bench.begin` post — XCTest
/// may invoke the measure block more than once, so the program re-arms
/// after every `dev.bench.done`. Fire-and-forget from initState; the widget
/// still runs it even after unmount (a detached controller just no-ops).
void _autoDrive(ScrollController controller) {
  () async {
    while (true) {
      await _awaitBegin();
      await _ScrollDriver.fling(controller);
      await _postDone();
      // Drop ack-race backlog: a begin that latched while the fling ran is
      // not a new window (the runner stops reposting once it sees done).
      await _configChannel.invokeMethod('discardBegins');
    }
  }();
}

class _ScrollDriver {
  static Future<void> fling(ScrollController c) async {
    if (!c.hasClients) return;
    final max = c.position.maxScrollExtent;
    await Future.delayed(const Duration(seconds: 1));
    for (var step = 1; step <= 8; step++) {
      await c.animateTo(max * step / 8,
          duration: const Duration(milliseconds: 900), curve: Curves.easeOut);
      await Future.delayed(const Duration(milliseconds: 250));
    }
    for (var step = 6; step >= 0; step -= 3) {
      await c.animateTo(max * step / 8,
          duration: const Duration(milliseconds: 900), curve: Curves.easeOut);
      await Future.delayed(const Duration(milliseconds: 250));
    }
  }
}

// MARK: - W2 Feed

class FeedPage extends StatefulWidget {
  const FeedPage({super.key, required this.autoDrive});
  final bool autoDrive;
  @override
  State<FeedPage> createState() => _FeedPageState();
}

class _FeedPageState extends State<FeedPage> {
  final _controller = ScrollController();
  @override
  void initState() {
    super.initState();
    _ready();
    if (widget.autoDrive) _autoDrive(_controller);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: ListView.builder(
        controller: _controller,
        itemCount: 10000,
        itemBuilder: (context, i) => Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
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
                    Text('Row title $i'),
                    Text('Second line of subtitle for item $i',
                        style: Theme.of(context).textTheme.bodySmall),
                  ],
                ),
              ),
              Text(timestamp(i),
                  style: Theme.of(context).textTheme.bodySmall),
            ],
          ),
        ),
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
    initRng.next();
    x = initRng.next() * (_fieldW - _rectSize);
    y = initRng.next() * (_fieldH - _rectSize);
    rot = initRng.next() * 360;
    op = 0.3 + initRng.next() * 0.7;
    _rng = XorShift(0x9E3779B97F4A7C15 ^ i * 0xBF58476D1CE4E5B9);
    _duration = Duration(milliseconds: 1200 + (i % 5) * 200);
    _timer = Timer.periodic(_duration, (_) => _step());
  }

  void _step() {
    setState(() {
      x = _rng.next() * (_fieldW - _rectSize);
      y = _rng.next() * (_fieldH - _rectSize);
      rot = _rng.next() * 360;
      op = 0.3 + _rng.next() * 0.7;
    });
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

class TextPage extends StatefulWidget {
  const TextPage({super.key, required this.autoDrive});
  final bool autoDrive;
  @override
  State<TextPage> createState() => _TextPageState();
}

class _TextPageState extends State<TextPage> {
  final _controller = ScrollController();
  @override
  void initState() {
    super.initState();
    _ready();
    if (widget.autoDrive) _autoDrive(_controller);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: ListView.builder(
        controller: _controller,
        itemCount: 50,
        itemBuilder: (context, i) => Padding(
          padding:
              const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
          child: Text(paragraphs[i % paragraphs.length]),
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
  const MotionCapacityPage({super.key, required this.autoDrive, this.pinnedStep});
  final bool autoDrive;
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
    if (widget.autoDrive && widget.pinnedStep == null) {
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
class FeedCapacityPage extends StatefulWidget {
  const FeedCapacityPage({super.key, required this.autoDrive, this.pinnedStep});
  final bool autoDrive;
  final int? pinnedStep;
  static const steps = [1, 2, 4, 8, 16, 32, 64];
  @override
  State<FeedCapacityPage> createState() => _FeedCapacityPageState();
}

class _FeedCapacityPageState extends State<FeedCapacityPage> {
  final _controller = ScrollController();
  late int _complexity = widget.pinnedStep ?? FeedCapacityPage.steps.first;

  @override
  void initState() {
    super.initState();
    _ready();
    if (widget.autoDrive && widget.pinnedStep == null) {
      () async {
        while (true) {
          await _awaitBegin();
          for (var i = 0; i < FeedCapacityPage.steps.length; i++) {
            setState(() => _complexity = FeedCapacityPage.steps[i]);
            await _logStep(i, _complexity);
            await Future.delayed(const Duration(seconds: 1));
            if (!_controller.hasClients) continue;
            final max = _controller.position.maxScrollExtent;
            for (var sweep = 0; sweep < 2; sweep++) {
              await _controller.animateTo(max,
                  duration: const Duration(milliseconds: 900),
                  curve: Curves.easeOut);
              await Future.delayed(const Duration(milliseconds: 100));
              await _controller.animateTo(0,
                  duration: const Duration(milliseconds: 900),
                  curve: Curves.easeOut);
              await Future.delayed(const Duration(milliseconds: 100));
            }
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
        controller: _controller,
        itemCount: 10000,
        itemBuilder: (context, i) => Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
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
                    Text('Row title $i'),
                    Text('Second line of subtitle for item $i',
                        style: Theme.of(context).textTheme.bodySmall),
                  ],
                ),
              ),
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
                      style: Theme.of(context).textTheme.labelSmall),
                ]),
              const SizedBox(width: 8),
              Text(timestamp(i),
                  style: Theme.of(context).textTheme.bodySmall),
            ],
          ),
        ),
      ),
    );
  }
}
