// Competitive benchmark — React Native contestant, workloads w1–w6
// (exact lowercase ids), per benchmarks/competitive/WORKLOADS.md. The
// workload and the capacity `step` arrive as initial props from the
// native side, which reads `-bench-workload` launch arguments from
// NSUserDefaults' NSArgumentDomain (Apple) or intent extras (Android).
// One pacing model on every leg: one launch renders one ladder step.
// Scrolling is driven from outside the app by OS-level input — the app
// never scrolls itself.

import React, { useEffect, useRef, useState } from 'react';
import {
  Animated,
  Easing,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from 'react-native';

const ROW_COLORS = [
  '#3B82F6', '#10B981', '#F59E0B', '#EF4444', '#8B5CF6', '#EC4899',
];

const PARAGRAPHS = [
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
      <Text style={{ fontSize: 20 }}>{`Count: ${count}`}</Text>
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

// MARK: - W2 Feed

const FEED_ROWS = Array.from({ length: 10_000 }, (_, i) => i);

// A FlatList is the idiomatic RN feed: it virtualizes rows like the
// UITableView/UICollectionView in the native contestants, instead of
// mounting all 10k rows into a ScrollView at once.
const FeedRow = React.memo(function FeedRow({ i, extra }) {
  return (
    <View style={styles.row}>
      <View
        style={[styles.avatar, { backgroundColor: ROW_COLORS[i % 6] }]}
      />
      <View style={{ flex: 1, marginLeft: 12 }}>
        <Text style={styles.rowTitle}>{`Row title ${i}`}</Text>
        <View style={{ height: 4 }} />
        <Text
          style={
            styles.rowSub
          }>{`Second line of subtitle for item ${i}`}</Text>
      </View>
      {extra}
      <View style={{ width: 12 }} />
      <Text style={styles.rowTime}>{timestamp(i)}</Text>
    </View>
  );
});

function Feed() {
  return (
    <View style={styles.fill}>
      <Animated.FlatList
        data={FEED_ROWS}
        renderItem={({ item }) => <FeedRow i={item} />}
        keyExtractor={i => String(i)}
        style={styles.fill}
      />
    </View>
  );
}

// MARK: - W5/W6 Capacity ladders

// One pacing model on every leg: one launch renders one step, pinned by
// the `step` prop. The host driver ends every cell on its own
// schedule — the app signals readiness only, never completion.

// W5: W3's scene with the rect count pinned per launch (200…25600).
const W5_STEPS = [200, 400, 800, 1600, 3200, 6400, 12800, 25600];

function MotionCapacity({ step }) {
  const [count] = useState(step);
  return (
    <View style={styles.fieldWrap}>
      <View style={{ width: FIELD_W, height: FIELD_H }}>
        {Array.from({ length: count }, (_, i) => (
          <MotionRect key={i} index={i} />
        ))}
      </View>
    </View>
  );
}

// W6: W2's feed with `complexity` nested text+shape cells per row
// (1…64); the runner's OS-level fling program runs during each hold.
const W6_STEPS = [1, 2, 4, 8, 16, 32, 64];

function FeedCapacity({ step }) {
  const [complexity] = useState(step);
  return (
    <View style={styles.fill}>
      <Animated.FlatList
        style={styles.fill}
        data={FEED_ROWS}
        extraData={complexity}
        keyExtractor={i => String(i)}
        renderItem={({ item: i }) => (
          <FeedRow i={i} extra={Array.from({ length: complexity }, (_, j) => (
            // cells separated by 4; the first carries the group's 12
            // gap to the text column (spec)
            <View key={j} style={{ alignItems: 'center', marginLeft: j === 0 ? 12 : 4 }}>
              <View
                style={{
                  width: 14,
                  height: 14,
                  borderRadius: 4,
                  backgroundColor: ROW_COLORS[(i + j) % 6],
                }}
              />
              <Text style={{ fontSize: 12, color: '#666' }}>{`c${j}`}</Text>
            </View>
          ))} />
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
        // one easing curve for every contestant:
        // cubic-bezier(0.42, 0.0, 0.58, 1.0)
        easing: Easing.bezier(0.42, 0.0, 0.58, 1.0),
        useNativeDriver: true,
      }).start(({ finished }) => {
        if (finished && alive) step();
      });
    };
    // Retarget at t=0 (the first draw eases init pose → first target),
    // then again each time this rect's own animation completes.
    step();
    return () => {
      alive = false;
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
    <View style={styles.fieldWrap}>
      <View style={{ width: FIELD_W, height: FIELD_H }}>
        {Array.from({ length: 200 }, (_, i) => (
          <MotionRect key={i} index={i} />
        ))}
      </View>
    </View>
  );
}

// MARK: - W4 Text

// All 50 paragraphs mount eagerly — layout cost is part of the measure.
function TextBench() {
  return (
    <View style={styles.fill}>
      <Animated.ScrollView style={styles.fill}>
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

export default function App({ workload, step }) {
  // Workload ids are the exact lowercase strings w1..=w6 — any other
  // value (including uppercase) fails.
  if (!['w1', 'w2', 'w3', 'w4', 'w5', 'w6'].includes(workload)) {
    throw new Error(
      `missing or unrecognized -bench-workload launch argument ` +
        `(got ${workload ?? 'null'}); expected w1..=w6`,
    );
  }
  // W5/W6: a `step` prop pins one ladder step per launch on every leg —
  // missing or malformed fails.
  const ladder = workload === 'w5' ? W5_STEPS : W6_STEPS;
  if (workload === 'w5' || workload === 'w6') {
    if (step == null || !ladder.includes(step)) {
      throw new Error(
        `missing or unrecognized -bench-step value ${step} for ` +
          `${workload}; expected one of ${ladder}`,
      );
    }
  }
  const page =
    workload === 'w2' ? (
      <Feed />
    ) : workload === 'w3' ? (
      <Motion />
    ) : workload === 'w4' ? (
      <TextBench />
    ) : workload === 'w5' ? (
      <MotionCapacity step={step} />
    ) : workload === 'w6' ? (
      <FeedCapacity step={step} />
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
  // field placement: pinned to the top of the content area with a
  // 16-point inset on mobile, centred on desktop (WORKLOADS.md)
  fieldWrap:
    Platform.OS === 'ios' || Platform.OS === 'android'
      ? { flex: 1, alignItems: 'center', paddingTop: 16 }
      : { flex: 1, alignItems: 'center', justifyContent: 'center' },
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
    paddingVertical: 10,
  },
  avatar: { width: 40, height: 40, borderRadius: 20 },
  rowTitle: { fontSize: 16, color: '#111' },
  rowSub: { fontSize: 13, color: '#666' },
  rowTime: { fontSize: 13, color: '#666', marginLeft: 12 },
  rect: { position: 'absolute', left: 0, top: 0, width: RECT, height: RECT },
  paragraph: {
    paddingHorizontal: 16,
    paddingVertical: 10,
    marginBottom: 6,
    fontSize: 16,
  },
});
