// Competitive benchmark — Flutter contestant, workloads w1–w6 (exact
// lowercase ids), per benchmarks/competitive/WORKLOADS.md. Selection:
// `-bench-workload w1|w2|w3|w4|w5|w6` (and `-bench-step N` for w5/w6),
// read through NSUserDefaults' NSArgumentDomain via the bench/config
// channel. One pacing model on every leg: one launch renders one ladder
// step. Scrolling is driven from outside the app by OS-level input —
// the app never scrolls itself. The host driver ends every cell on
// its own schedule — the app posts readiness only, never completion.

import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

const _configChannel = MethodChannel('bench/config');

/// Apple legs only: the native side posts `dev.bench.ready.<bundle>.<w>`
/// over Darwin notify when Dart reports the workload page's first frame.
const _readyChannel = MethodChannel('bench/ready');

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final workload = await _readConfig('workload');
  if (!const ['w1', 'w2', 'w3', 'w4', 'w5', 'w6'].contains(workload)) {
    throw StateError(
        'missing or unrecognized -bench-workload launch argument '
        '(got ${workload ?? 'null'}); expected w1..=w6');
  }
  final stepStr = await _readConfig('step');
  final pinnedStep = stepStr == null ? null : int.tryParse(stepStr);
  if (stepStr != null && pinnedStep == null) {
    throw StateError('malformed -bench-step value $stepStr; expected integer');
  }
  if (const ['w5', 'w6'].contains(workload)) {
    final ladder = workload == 'w5'
        ? MotionCapacityPage.steps
        : FeedCapacityPage.steps;
    if (pinnedStep == null) {
      throw StateError(
          '$workload requires -bench-step (or BENCH_STEP); every leg '
          'measures one ladder step per launch');
    }
    if (!ladder.contains(pinnedStep)) {
      throw StateError(
          'unrecognized -bench-step value $pinnedStep for $workload; '
          'expected one of $ladder');
    }
  }
  runApp(BenchApp(workload: workload!, pinnedStep: pinnedStep));
}

/// Reads a benchmark config value. On Apple targets the platform channel
/// serves it from NSUserDefaults' NSArgumentDomain (launch arguments);
/// on Android the override MainActivity serves the same channel from
/// intent extras; desktop legs pass `BENCH_<NAME>` in the environment.
/// A missing channel key falls through to the environment; any other
/// error propagates — never a silent fallback.
Future<String?> _readConfig(String name) async {
  try {
    final v = await _configChannel.invokeMethod<String>(name);
    if (v != null) return v;
  } on MissingPluginException {
    // No native bench/config channel on this platform — fall through to
    // the environment.
  }
  return Platform.environment['BENCH_${name.toUpperCase()}'];
}

class BenchApp extends StatelessWidget {
  const BenchApp({super.key, required this.workload, this.pinnedStep});
  final String workload;

  /// The per-launch capacity protocol: one ladder step per app start,
  /// arriving as the `step` config value. Null for non-capacity
  /// workloads; a capacity launch always carries one on every platform.
  final int? pinnedStep;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Bench',
      home: switch (workload) {
        'w1' => const HelloPage(),
        'w2' => const FeedPage(),
        'w3' => const MotionPage(),
        'w4' => const TextPage(),
        'w5' => MotionCapacityPage(pinnedStep: pinnedStep),
        'w6' => FeedCapacityPage(pinnedStep: pinnedStep),
        _ => throw StateError('unreachable: workload() yields w1..=w6'),
      },
    );
  }
}

bool _readyPosted = false;

/// Readiness = the workload page's first frame, the point every Apple
/// contestant posts `dev.bench.ready` at (SwiftUI `onAppear`, UIKit /
/// AppKit `viewDidAppear`, WaterUI `on_appear`, React Native's first
/// content appearance). Only the Apple legs consume it — the other legs
/// observe the first present from outside the app.
void _ready() {
  if (_readyPosted || !(Platform.isIOS || Platform.isMacOS)) return;
  _readyPosted = true;
  SchedulerBinding.instance.addPostFrameCallback(
      (_) => _readyChannel.invokeMethod<void>('ready'));
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
            Text('Count: $counter', style: const TextStyle(fontSize: 20)),
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
              const SizedBox(height: 4),
              Text('Second line of subtitle for item $i',
                  style: TextStyle(
                      fontSize: 13, color: Colors.grey.shade600)),
            ],
          ),
        ),
        if (extra.isNotEmpty) const SizedBox(width: 12),
        for (var k = 0; k < extra.length; k++) ...[
          extra[k],
          if (k + 1 < extra.length) const SizedBox(width: 4),
        ],
        const SizedBox(width: 12),
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

// Field placement: pinned to the top of the content area with a
// 16-point inset on mobile, centred on desktop (WORKLOADS.md).
Widget _fieldPlacement(Widget field) {
  final mobile = Platform.isAndroid || Platform.isIOS;
  if (!mobile) {
    return Center(child: field);
  }
  return Padding(
    padding: const EdgeInsets.only(top: 16),
    child: Align(alignment: Alignment.topCenter, child: field),
  );
}

class MotionPage extends StatelessWidget {
  const MotionPage({super.key});
  @override
  Widget build(BuildContext context) {
    _ready();
    return Scaffold(
      body: _fieldPlacement(
        SizedBox(
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
  late double x, y, rot, prevRot, op;
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
    prevRot = rot;
    op = 0.3 + initRng.next() * 0.7;
    _rng = XorShift(0x9E3779B97F4A7C15 ^ i * 0xBF58476D1CE4E5B9);
    _duration = Duration(milliseconds: 1200 + (i % 5) * 200);
    // Retarget at t=0 — the first setState lands in the frame AFTER the
    // init pose is drawn, so the rect eases init → first target like every
    // other contestant (no replaced first pose, no discarded draws).
    _timer = Timer(_duration * 0, () {
      if (!mounted) return;
      _step();
      _timer = Timer.periodic(_duration, (_) => _step());
    });
  }

  /// Pulls the next drive-stream target; every retarget goes through
  /// setState so the implicit animation plays the transition.
  void _step() {
    setState(() {
      x = _rng.next() * (_fieldW - _rectSize);
      y = _rng.next() * (_fieldH - _rectSize);
      prevRot = rot;
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
          // previous target -> current target: the first build draws
          // the init pose (prevRot == rot), the t=0 retarget animates it
          tween: Tween(begin: prevRot, end: rot),
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
            for (var i = 0; i < 50; i++) ...[
              Padding(
                padding: const EdgeInsets.symmetric(
                    horizontal: 16, vertical: 10),
                child: Text(paragraphs[i % paragraphs.length],
                    style: const TextStyle(fontSize: 16)),
              ),
              const SizedBox(height: 6),
            ],
          ],
        ),
      ),
    );
  }
}

// MARK: - W5 Motion capacity

/// W3's scene with the rect count pinned per launch (200…25600); the
/// host driver ends the cell on its own schedule.
class MotionCapacityPage extends StatefulWidget {
  const MotionCapacityPage({super.key, this.pinnedStep});
  final int? pinnedStep;
  static const steps = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];
  @override
  State<MotionCapacityPage> createState() => _MotionCapacityPageState();
}

class _MotionCapacityPageState extends State<MotionCapacityPage> {
  late final int _count = widget.pinnedStep!;

  @override
  void initState() {
    super.initState();
    _ready();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: _fieldPlacement(
        SizedBox(
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
  late final int _complexity = widget.pinnedStep!;

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
