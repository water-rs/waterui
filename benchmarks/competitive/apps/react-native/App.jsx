// Competitive benchmark — React Native contestant, workloads W1–W6.
// Workload and drive mode arrive as initial props from the native side,
// which reads `-bench-workload`/`-bench-drive` launch arguments from
// NSUserDefaults' NSArgumentDomain (identical on iOS and macOS).

import React, { useEffect, useRef, useState } from 'react';
import {
  Animated,
  Easing,
  NativeModules,
  Pressable,
  StyleSheet,
  Text,
  View,
} from 'react-native';

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

// Deterministic PRNG — same xorshift64 as every other contestant.
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

function Hello() {
  const [count, setCount] = useState(0);
  return (
    <View style={styles.center}>
      <Text>{`Count: ${count}`}</Text>
      <View style={{ height: 16 }} />
      <Pressable
        accessibilityIdentifier="increment-button"
        testID="increment-button"
        accessibilityLabel="Increment"
        style={styles.button}
        onPress={() => setCount(c => c + 1)}>
        <Text style={styles.buttonLabel}>Increment</Text>
      </Pressable>
    </View>
  );
}

// MARK: - Scroll self-drive (identical fling program on every platform)

// `auto` mode waits for the runner's `dev.bench.begin` Darwin
// notification inside its measure block — polled through the BenchNotify
// native module, not AX (a timed-out AX query fails the test instead of
// driving it) — then drives the ScrollView through the ref: 8 ease-out
// bursts to the bottom, then 2 back to the top, each 0.9 s + 0.25 s
// settle, and posts `dev.bench.done` when the program finishes. The
// program never self-starts at mount.
function useScrollDrive(enabled) {
  const scrollRef = useRef(null);
  const extent = useRef({ max: 0, pos: 0, viewH: 0 });
  const [ready, setReady] = useState(false);
  const [began, setBegan] = useState(false);
  const beganRef = useRef(false);
  // Re-arms after every program: XCTest may invoke the measure block more
  // than once, so the poll keeps running and each observed post starts a
  // fresh program. The poll is gated on `beganRef` so a post arriving
  // mid-program stays latched for the next window instead of being lost.
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const timer = setInterval(async () => {
      if (beganRef.current) return;
      try {
        if (await NativeModules.BenchNotify.beginObserved()) {
          beganRef.current = true;
          if (!cancelled) setBegan(true);
        }
      } catch {}
    }, 50);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [enabled]);
  useEffect(() => {
    if (!enabled || !began || !ready || extent.current.max <= 0) return;
    const view = scrollRef.current;
    if (!view) return;
    const max = extent.current.max;
    const steps = [1, 2, 3, 4, 5, 6, 7, 8, 6, 3, 0].map(s => (max * s) / 8);
    let cancelled = false;
    const sleep = ms => new Promise(r => setTimeout(r, ms));
    (async () => {
      await sleep(1000);
      for (const v of steps) {
        if (cancelled) return;
        // RN's scrollTo cannot take an easing+duration; step it smoothly.
        const from = extent.current.pos;
        const t0 = Date.now();
        while (!cancelled) {
          const t = Math.min((Date.now() - t0) / 900, 1);
          const e = 1 - Math.pow(1 - t, 3); // easeOutCubic
          scrollY(view, from + (v - from) * e);
          if (t >= 1) break;
          await sleep(16);
        }
        await sleep(250);
      }
      if (!cancelled) {
        NativeModules.BenchNotify.postDone();
        // Drop ack-race backlog: a begin latched while the program ran is
        // not a new window (the runner stops reposting once it sees done).
        NativeModules.BenchNotify.discardBegins();
        beganRef.current = false;
        setBegan(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled, began, ready]);
  const updateMax = (contentH, viewH) => {
    extent.current.max = Math.max(contentH - viewH, 0);
    if (extent.current.max > 0) setReady(true);
  };
  return {
    ref: scrollRef,
    onContentSizeChange: (w, h) => updateMax(h, extent.current.viewH),
    onLayout: e => {
      extent.current.viewH = e.nativeEvent.layout.height;
      updateMax(extent.current.contentH ?? 0, extent.current.viewH);
    },
    onScroll: e => {
      extent.current.pos = e.nativeEvent.contentOffset.y;
      extent.current.contentH = e.nativeEvent.contentSize.height;
    },
    scrollEventThrottle: 16,
  };
}

// MARK: - W2 Feed

const FEED_ROWS = Array.from({ length: 10_000 }, (_, i) => i);

// A FlatList is the idiomatic RN feed: it virtualizes rows like the
// UITableView/UICollectionView in the native contestants, instead of
// mounting all 10k rows into a ScrollView at once.
const FeedRow = React.memo(function FeedRow({ i }) {
  return (
    <View style={styles.row}>
      <View
        style={[styles.avatar, { backgroundColor: ROW_COLORS[i % 6] }]}
      />
      <View style={{ flex: 1, marginLeft: 12 }}>
        <Text style={styles.rowTitle}>{`Row title ${i}`}</Text>
        <Text
          style={
            styles.rowSub
          }>{`Second line of subtitle for item ${i}`}</Text>
      </View>
      <Text style={styles.rowTime}>{timestamp(i)}</Text>
    </View>
  );
});

// scrollToOffset on FlatList takes {offset}; a ScrollView ref takes {y}.
// Same easing program either way.
function scrollY(view, y) {
  if (view.scrollToOffset) {
    view.scrollToOffset({ offset: y, animated: false });
  } else {
    view.scrollTo({ y, animated: false });
  }
}

function Feed({ autoDrive }) {
  const scrollProps = useScrollDrive(autoDrive);
  return (
    <View style={styles.fill}>
      <Animated.FlatList
        {...scrollProps}
        data={FEED_ROWS}
        renderItem={({ item }) => <FeedRow i={item} />}
        keyExtractor={i => String(i)}
        style={styles.fill}
      />
    </View>
  );
}

// MARK: - W5/W6 Capacity ladders

// Waits for one `dev.bench.begin` cycle through the native module — the
// same handshake the scroll self-drive uses.
function useCapacityDrive(autoDrive, runLadder) {
  const armedRef = useRef(false);
  const runRef = useRef(runLadder);
  runRef.current = runLadder;
  useEffect(() => {
    if (!autoDrive) return;
    let cancelled = false;
    const sleep = ms => new Promise(r => setTimeout(r, ms));
    (async () => {
      while (!cancelled) {
        try {
          if (await NativeModules.BenchNotify.beginObserved()) {
            await runRef.current();
            NativeModules.BenchNotify.postDone();
            // Drop ack-race backlog before re-arming.
            NativeModules.BenchNotify.discardBegins();
          }
        } catch {}
        await sleep(50);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoDrive]);
}

// `step k n=<param> t=<unix>` → bench-steps.log + dev.bench.step post —
// the runner slices its xctrace recording by these timestamps.
const logStep = (step, n) =>
  NativeModules.BenchNotify.logStep(step, n);

// W5: W3's scene with the rect count doubled per step (200…25600);
// 5 s per step once the runner posts `dev.bench.begin`.
const W5_STEPS = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];

function MotionCapacity({ autoDrive }) {
  const [count, setCount] = useState(W5_STEPS[0]);
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  useCapacityDrive(autoDrive, async () => {
    for (let i = 0; i < W5_STEPS.length; i++) {
      setCount(W5_STEPS[i]);
      logStep(i, W5_STEPS[i]);
      await sleep(5000);
    }
  });
  return (
    <View style={styles.center}>
      <View style={{ width: FIELD_W, height: FIELD_H }}>
        {Array.from({ length: count }, (_, i) => (
          <MotionRect key={i} index={i} />
        ))}
      </View>
    </View>
  );
}

// W6: W2's fling program over rows with `complexity` nested text+shape
// children (1…64); two full sweeps inside each 5 s hold.
const W6_STEPS = [1, 2, 4, 8, 16, 32, 64];

function FeedCapacity({ autoDrive }) {
  const [complexity, setComplexity] = useState(W6_STEPS[0]);
  const scrollRef = useRef(null);
  const extent = useRef({ max: 0, pos: 0 });
  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const sweep = async to => {
    const view = scrollRef.current;
    if (!view) return;
    const from = extent.current.pos;
    const t0 = Date.now();
    while (true) {
      const t = Math.min((Date.now() - t0) / 900, 1);
      const e = 1 - Math.pow(1 - t, 3);
      scrollY(view, from + (to - from) * e);
      if (t >= 1) break;
      await sleep(16);
    }
    await sleep(100);
  };
  useCapacityDrive(autoDrive, async () => {
    for (let i = 0; i < W6_STEPS.length; i++) {
      setComplexity(W6_STEPS[i]);
      logStep(i, W6_STEPS[i]);
      await sleep(1000);
      for (let k = 0; k < 2; k++) {
        await sweep(extent.current.max);
        await sweep(0);
      }
    }
  });
  return (
    <View style={styles.fill}>
      <Animated.FlatList
        ref={scrollRef}
        style={styles.fill}
        data={FEED_ROWS}
        extraData={complexity}
        keyExtractor={i => String(i)}
        onContentSizeChange={(w, h) => {
          extent.current.max = Math.max(
            h - (extent.current.viewH ?? 0), 0);
        }}
        onLayout={e => {
          extent.current.viewH = e.nativeEvent.layout.height;
        }}
        onScroll={e => {
          extent.current.pos = e.nativeEvent.contentOffset.y;
          extent.current.max = Math.max(
            e.nativeEvent.contentSize.height -
              e.nativeEvent.layoutMeasurement.height, 0);
        }}
        scrollEventThrottle={16}
        renderItem={({ item: i }) => (
          <View style={styles.row}>
            <View
              style={[styles.avatar, { backgroundColor: ROW_COLORS[i % 6] }]}
            />
            <View style={{ flex: 1, marginLeft: 12 }}>
              <Text style={styles.rowTitle}>{`Row title ${i}`}</Text>
              <Text
                style={
                  styles.rowSub
                }>{`Second line of subtitle for item ${i}`}</Text>
            </View>
            {Array.from({ length: complexity }, (_, j) => (
              <View key={j} style={{ alignItems: 'center', marginLeft: 4 }}>
                <View
                  style={{
                    width: 14,
                    height: 14,
                    borderRadius: 4,
                    backgroundColor: ROW_COLORS[(i + j) % 6],
                  }}
                />
                <Text style={{ fontSize: 9, color: '#666' }}>{`c${j}`}</Text>
              </View>
            ))}
            <Text style={styles.rowTime}>{timestamp(i)}</Text>
          </View>
        )}
      />
    </View>
  );
}

// MARK: - W3 Motion

const FIELD_W = 720;
const FIELD_H = 440;
const RECT = 40;

// One rect: a pose (x, y, rotation, opacity) animating toward a new random
// target every `duration` ms, ease-in-out — the same program as every other
// contestant.
function MotionRect({ index }) {
  const cfg = useRef(null);
  if (cfg.current === null) {
    const init = makeXorShift(
      0xd1b54a32d192ed03n ^ BigInt(index) * 0x2545f4914f6cdd1dn,
    );
    init();
    const rng = makeXorShift(
      0x9e3779b97f4a7c15n ^ BigInt(index) * 0xbf58476d1ce4e5b9n,
    );
    cfg.current = {
      duration: 1200 + (index % 5) * 200,
      initial: {
        x: init() * (FIELD_W - RECT),
        y: init() * (FIELD_H - RECT),
        rot: init() * 360,
        op: 0.3 + init() * 0.7,
      },
      next: () => ({
        x: rng() * (FIELD_W - RECT),
        y: rng() * (FIELD_H - RECT),
        rot: rng() * 360,
        op: 0.3 + rng() * 0.7,
      }),
    };
  }
  const { duration, initial, next } = cfg.current;
  const [poses, setPoses] = useState({ from: initial, to: initial });
  const poseRef = useRef(initial);
  const anim = useRef(new Animated.Value(1)).current;

  useEffect(() => {
    let alive = true;
    const step = () => {
      if (!alive) return;
      const target = next();
      const prev = poseRef.current;
      poseRef.current = target;
      setPoses({ from: prev, to: target });
      anim.setValue(0);
      Animated.timing(anim, {
        toValue: 1,
        duration,
        easing: Easing.inOut(Easing.ease),
        useNativeDriver: true,
      }).start(({ finished }) => {
        if (finished && alive) step();
      });
    };
    const t = setTimeout(step, duration);
    return () => {
      alive = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const { from, to } = poses;
  return (
    <Animated.View
      style={[
        styles.rect,
        {
          opacity: anim.interpolate({
            inputRange: [0, 1],
            outputRange: [from.op, to.op],
          }),
          transform: [
            {
              translateX: anim.interpolate({
                inputRange: [0, 1],
                outputRange: [from.x, to.x],
              }),
            },
            {
              translateY: anim.interpolate({
                inputRange: [0, 1],
                outputRange: [from.y, to.y],
              }),
            },
            {
              rotate: anim.interpolate({
                inputRange: [0, 1],
                outputRange: [`${from.rot}deg`, `${to.rot}deg`],
              }),
            },
          ],
        },
      ]}>
      <View
        style={{
          width: RECT,
          height: RECT,
          borderRadius: 10,
          backgroundColor: ROW_COLORS[index % 6],
        }}
      />
    </Animated.View>
  );
}

function Motion() {
  return (
    <View style={styles.center}>
      <View style={{ width: FIELD_W, height: FIELD_H }}>
        {Array.from({ length: 200 }, (_, i) => (
          <MotionRect key={i} index={i} />
        ))}
      </View>
    </View>
  );
}

// MARK: - W4 Text

function TextBench({ autoDrive }) {
  const scrollProps = useScrollDrive(autoDrive);
  return (
    <View style={styles.fill}>
      <Animated.ScrollView {...scrollProps} style={styles.fill}>
        {Array.from({ length: 50 }, (_, i) => (
          <Text key={i} style={styles.paragraph}>
            {PARAGRAPHS[i % PARAGRAPHS.length]}
          </Text>
        ))}
      </Animated.ScrollView>
    </View>
  );
}

// MARK: - Root

export default function App({ workload, drive }) {
  // The native side traps on a missing/unrecognized -bench-workload; this
  // guard keeps the same contract if JS ever runs without it.
  if (!['W1', 'W2', 'W3', 'W4', 'W5', 'W6'].includes(workload)) {
    throw new Error(
      `missing or unrecognized -bench-workload launch argument ` +
        `(got ${workload ?? 'null'}); expected W1..=W6`,
    );
  }
  if (!['swipe', 'auto'].includes(drive)) {
    throw new Error(
      `unrecognized -bench-drive value ${drive}; expected swipe|auto`,
    );
  }
  const autoDrive = drive === 'auto';
  const page =
    workload === 'W2' ? (
      <Feed autoDrive={autoDrive} />
    ) : workload === 'W3' ? (
      <Motion />
    ) : workload === 'W4' ? (
      <TextBench autoDrive={autoDrive} />
    ) : workload === 'W5' ? (
      <MotionCapacity autoDrive={autoDrive} />
    ) : workload === 'W6' ? (
      <FeedCapacity autoDrive={autoDrive} />
    ) : (
      <Hello />
    );
  // The runner asserts this accessibility identifier after launch.
  return (
    <View style={styles.fill}>
      {/* The runner asserts this accessibility identifier after launch. A
          plain container View may never enter the AX tree, so the id rides
          on a dedicated 1x1 accessible element. */}
      <Text
        style={{ position: 'absolute', width: 1, height: 1, top: 0, left: 0, fontSize: 1 }}
        accessibilityIdentifier={`bench-workload-${workload}`}
        testID={`bench-workload-${workload}`}>
        {`bench-workload-${workload}`}
      </Text>
      {page}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center' },
  button: {
    backgroundColor: '#0A84FF',
    borderRadius: 8,
    paddingHorizontal: 20,
    paddingVertical: 10,
  },
  buttonLabel: { color: '#fff', fontWeight: '600' },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 4,
  },
  avatar: { width: 40, height: 40, borderRadius: 20 },
  rowTitle: { fontSize: 16, color: '#111' },
  rowSub: { fontSize: 13, color: '#666' },
  rowTime: { fontSize: 13, color: '#666', marginLeft: 12 },
  rect: { position: 'absolute', left: 0, top: 0, width: RECT, height: RECT },
  paragraph: { paddingHorizontal: 16, paddingVertical: 6, fontSize: 17 },
});
