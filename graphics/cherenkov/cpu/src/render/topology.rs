//! Incremental winding compilation, independent of pixel rows and placement.
//!
//! An indexed treap carries winding sums and visibility. Adjacent pairs own
//! one indexed crossing event each; endpoint and crossing batches update only
//! changed prefixes. The output references original lines, never rounded x
//! intersections, and coalesces intervals until their visibility changes.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use cherenkov::FillRule;

use super::tiles::IntegerEdge;

const NONE: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
pub struct Visible {
    pub source: usize,
    pub top: i64,
    pub bottom: i64,
    pub sign: i32,
}

#[derive(Clone, Copy)]
struct Line {
    x: i64,
    y: i64,
    dx: i64,
    dy: i64,
}

impl Line {
    const fn of(e: IntegerEdge) -> (Self, i32) {
        let (x, y, dx, dy, sign) = if e.y0 < e.y1 {
            (e.x0, e.y0, e.x1 - e.x0, e.y1 - e.y0, 1)
        } else {
            (e.x1, e.y1, e.x0 - e.x1, e.y0 - e.y1, -1)
        };
        (Self { x, y, dx, dy }, sign)
    }

    fn key(self) -> (i64, i64, i128) {
        let (mut a, mut b) = (self.dx.abs(), self.dy);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let (dx, dy) = (self.dx / a, self.dy / a);
        (
            dx,
            dy,
            i128::from(self.x) * i128::from(dy) - i128::from(self.y) * i128::from(dx),
        )
    }

    /// Order immediately before y+1, including every crossing floored to y.
    fn compare(self, other: Self, y: i64) -> Ordering {
        let (ad, bd) = (i128::from(self.dy), i128::from(other.dy));
        let (ax, bx) = (i128::from(self.dx), i128::from(other.dx));
        let distance = i128::from(self.x - other.x) * ad * bd
            + i128::from(y + 1 - self.y) * ax * bd
            - i128::from(y + 1 - other.y) * bx * ad;
        distance.cmp(&0).then_with(|| (bx * ad).cmp(&(ax * bd)))
    }

    fn crossing(self, other: Self) -> Option<i64> {
        let (ad, bd) = (i128::from(self.dy), i128::from(other.dy));
        let (ax, bx) = (i128::from(self.dx), i128::from(other.dx));
        let denominator = ax * bd - bx * ad;
        if denominator <= 0 {
            return None;
        }
        let numerator = i128::from(other.x - self.x) * ad * bd + i128::from(self.y) * ax * bd
            - i128::from(other.y) * bx * ad;
        Some(i64::try_from(numerator.div_euclid(denominator)).expect("crossing ordinate fits i64"))
    }
}

/// A normalized boundary waiting for its top endpoint. Treap state exists
/// only while the boundary intersects the current sweep ordinate.
#[derive(Clone, Copy)]
struct Pending {
    source: usize,
    top: i64,
    bottom: i64,
    weight: i32,
    active: usize,
}

struct Node {
    line: Line,
    source: usize,
    bottom: i64,
    weight: i32,
    left: usize,
    right: usize,
    parent: usize,
    previous: usize,
    next: usize,
    priority: u64,
    count: usize,
    sum: i32,
    min: i32,
    max: i32,
    visible: usize,
    incoming: i32,
    dirty: bool,
    sign: i32,
    opened: i64,
}

/// At most one queued event per active left neighbor. Updating adjacency
/// replaces its slot instead of retaining invalidated heap entries.
#[derive(Default)]
struct Events {
    heap: Vec<usize>,
    slots: Vec<usize>,
    keys: Vec<i64>,
}

impl Events {
    fn reset(&mut self) {
        self.heap.clear();
        self.slots.clear();
        self.keys.clear();
    }

    fn less(&self, a: usize, b: usize) -> bool {
        (self.keys[self.heap[a]], self.heap[a]) < (self.keys[self.heap[b]], self.heap[b])
    }

    fn swap(&mut self, a: usize, b: usize) {
        self.heap.swap(a, b);
        self.slots[self.heap[a]] = a;
        self.slots[self.heap[b]] = b;
    }

    fn repair(&mut self, mut at: usize) {
        while at != 0 && self.less(at, (at - 1) / 2) {
            self.swap(at, (at - 1) / 2);
            at = (at - 1) / 2;
        }
        loop {
            let left = at * 2 + 1;
            if left >= self.heap.len() {
                break;
            }
            let right = left + 1;
            let child = if right < self.heap.len() && self.less(right, left) {
                right
            } else {
                left
            };
            if !self.less(child, at) {
                break;
            }
            self.swap(child, at);
            at = child;
        }
    }

    fn set(&mut self, node: usize, key: Option<i64>) {
        let at = self.slots[node];
        if let Some(key) = key {
            self.keys[node] = key;
            if at == NONE {
                self.slots[node] = self.heap.len();
                self.heap.push(node);
                self.repair(self.heap.len() - 1);
            } else {
                self.repair(at);
            }
        } else if at != NONE {
            self.slots[node] = NONE;
            let last = self.heap.pop().expect("a queued event has a heap slot");
            if at < self.heap.len() {
                self.heap[at] = last;
                self.slots[last] = at;
                self.repair(at);
            }
        }
    }

    fn first(&self) -> Option<(i64, usize)> {
        self.heap.first().map(|&i| (self.keys[i], i))
    }
}

#[derive(Default)]
pub struct Sweep {
    nodes: Vec<Node>,
    free: usize,
    pending: Vec<Pending>,
    lines: Vec<((i64, i64, i128), usize)>,
    coincident: Vec<(i64, i32, usize)>,
    live: BinaryHeap<Reverse<(i64, usize)>>,
    endpoints: Vec<(i64, bool, usize)>,
    events: Events,
    root: usize,
    pub output: Vec<Visible>,
}

impl Sweep {
    pub fn boundaries(&mut self, edges: &[IntegerEdge], rule: FillRule) -> Vec<IntegerEdge> {
        self.compile(edges, rule);
        self.output
            .iter()
            .map(|v| {
                let mut edge = edges[v.source];
                if (edge.y0 < edge.y1) != (v.sign > 0) {
                    std::mem::swap(&mut edge.x0, &mut edge.x1);
                    std::mem::swap(&mut edge.y0, &mut edge.y1);
                }
                edge.top = v.top;
                edge.bottom = v.bottom;
                edge
            })
            .collect()
    }
    pub fn heap_bytes(&self) -> usize {
        self.nodes.capacity() * size_of::<Node>()
            + self.pending.capacity() * size_of::<Pending>()
            + self.lines.capacity() * size_of::<((i64, i64, i128), usize)>()
            + self.coincident.capacity() * size_of::<(i64, i32, usize)>()
            + self.live.capacity() * size_of::<Reverse<(i64, usize)>>()
            + self.endpoints.capacity() * size_of::<(i64, bool, usize)>()
            + (self.events.heap.capacity() + self.events.slots.capacity()) * size_of::<usize>()
            + self.events.keys.capacity() * size_of::<i64>()
            + self.output.capacity() * size_of::<Visible>()
    }

    fn add(&mut self, source: usize, top: i64, bottom: i64, weight: i32) {
        self.pending.push(Pending {
            source,
            top,
            bottom,
            weight,
            active: NONE,
        });
    }

    fn activate(&mut self, pending: usize, edges: &[IntegerEdge]) {
        let boundary = self.pending[pending];
        // Keep priorities tied to normalized boundary order, not recycled slots.
        let mut priority = pending as u64 + 1;
        priority = (priority ^ (priority >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        priority = (priority ^ (priority >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        priority ^= priority >> 31;
        let node = Node {
            line: Line::of(edges[boundary.source]).0,
            source: boundary.source,
            bottom: boundary.bottom,
            weight: boundary.weight,
            left: NONE,
            right: NONE,
            parent: NONE,
            previous: NONE,
            next: NONE,
            priority,
            count: 1,
            sum: boundary.weight,
            min: boundary.weight.min(0),
            max: boundary.weight.max(0),
            visible: 0,
            incoming: 0,
            dirty: true,
            sign: 0,
            opened: boundary.top,
        };
        let slot = if self.free == NONE {
            let slot = self.nodes.len();
            self.nodes.push(node);
            self.events.slots.push(NONE);
            self.events.keys.push(0);
            slot
        } else {
            let slot = self.free;
            self.free = self.nodes[slot].left;
            self.nodes[slot] = node;
            slot
        };
        self.pending[pending].active = slot;
        self.insert(slot, boundary.top);
    }

    /// Canonical supporting lines merge coincident winding before the sweep.
    fn normalize(&mut self, edges: &[IntegerEdge]) {
        self.pending.clear();
        self.lines.clear();
        for (i, &edge) in edges.iter().enumerate() {
            if edge.y0 != edge.y1 {
                self.lines.push((Line::of(edge).0.key(), i));
            }
        }
        self.lines.sort_unstable();
        let mut first = 0;
        while first < self.lines.len() {
            let mut last = first + 1;
            while last < self.lines.len() && self.lines[last].0 == self.lines[first].0 {
                last += 1;
            }
            if last == first + 1 {
                let source = self.lines[first].1;
                let edge = edges[source];
                self.add(
                    source,
                    edge.y0.min(edge.y1),
                    edge.y0.max(edge.y1),
                    Line::of(edge).1,
                );
            } else {
                self.coincident.clear();
                self.live.clear();
                for &(_, source) in &self.lines[first..last] {
                    let edge = edges[source];
                    let sign = Line::of(edge).1;
                    self.coincident.push((edge.y0.min(edge.y1), sign, source));
                    self.coincident.push((edge.y0.max(edge.y1), -sign, source));
                }
                self.coincident.sort_unstable();
                let (mut at, mut weight) = (0, 0);
                while at < self.coincident.len() {
                    let y = self.coincident[at].0;
                    while at < self.coincident.len() && self.coincident[at].0 == y {
                        let (_, change, source) = self.coincident[at];
                        weight += change;
                        let edge = edges[source];
                        if y == edge.y0.min(edge.y1) {
                            self.live.push(Reverse((edge.y0.max(edge.y1), source)));
                        }
                        at += 1;
                    }
                    while self.live.peek().is_some_and(|e| e.0.0 <= y) {
                        self.live.pop();
                    }
                    if weight != 0 && at < self.coincident.len() {
                        let source = self
                            .live
                            .peek()
                            .expect("nonzero winding has an active line")
                            .0
                            .1;
                        self.add(source, y, self.coincident[at].0, weight);
                    }
                }
            }
            first = last;
        }
    }

    pub fn compile(&mut self, edges: &[IntegerEdge], rule: FillRule) {
        self.output.clear();
        self.normalize(edges);
        self.endpoints.clear();
        for (i, node) in self.pending.iter().enumerate() {
            self.endpoints.push((node.top, true, i));
            self.endpoints.push((node.bottom, false, i));
        }
        self.endpoints.sort_unstable();
        self.nodes.clear();
        self.free = NONE;
        self.events.reset();
        self.root = NONE;
        let mut endpoint = 0;
        while endpoint < self.endpoints.len() || self.events.first().is_some() {
            let end_y = self.endpoints.get(endpoint).map_or(i64::MAX, |e| e.0);
            let y = self.events.first().map_or(end_y, |e| end_y.min(e.0));
            while endpoint < self.endpoints.len()
                && self.endpoints[endpoint].0 == y
                && !self.endpoints[endpoint].1
            {
                let slot = self.pending[self.endpoints[endpoint].2].active;
                self.remove(slot, y);
                // Removal cancels the slot's event and unlinks every neighbor.
                self.nodes[slot].left = self.free;
                self.free = slot;
                endpoint += 1;
            }
            while let Some((event_y, a)) = self.events.first() {
                if event_y != y {
                    break;
                }
                let b = self.nodes[a].next;
                self.events.set(a, None);
                self.exchange(a, b, y);
            }
            while endpoint < self.endpoints.len() && self.endpoints[endpoint].0 == y {
                self.activate(self.endpoints[endpoint].2, edges);
                endpoint += 1;
            }
            self.refresh(self.root, 0, y, rule);
        }
    }

    fn count(&self, i: usize) -> usize {
        if i == NONE { 0 } else { self.nodes[i].count }
    }
    fn sum(&self, i: usize) -> i32 {
        if i == NONE { 0 } else { self.nodes[i].sum }
    }

    fn pull(&mut self, i: usize) {
        let (l, r, weight) = (
            self.nodes[i].left,
            self.nodes[i].right,
            self.nodes[i].weight,
        );
        let prefix = self.sum(l) + weight;
        let (mut min, mut max) = (prefix.min(0), prefix.max(0));
        let mut visible = usize::from(self.nodes[i].sign != 0);
        if l != NONE {
            min = min.min(self.nodes[l].min);
            max = max.max(self.nodes[l].max);
            visible += self.nodes[l].visible;
            self.nodes[l].parent = i;
        }
        if r != NONE {
            min = min.min(prefix + self.nodes[r].min);
            max = max.max(prefix + self.nodes[r].max);
            visible += self.nodes[r].visible;
            self.nodes[r].parent = i;
        }
        let count = 1 + self.count(l) + self.count(r);
        let sum = prefix + self.sum(r);
        let node = &mut self.nodes[i];
        node.count = count;
        node.sum = sum;
        node.min = min;
        node.max = max;
        node.visible = visible;
        node.dirty = true;
    }

    fn merge(&mut self, a: usize, b: usize) -> usize {
        if a == NONE {
            return b;
        }
        if b == NONE {
            return a;
        }
        if self.nodes[a].priority < self.nodes[b].priority {
            self.nodes[a].right = self.merge(self.nodes[a].right, b);
            self.pull(a);
            a
        } else {
            self.nodes[b].left = self.merge(a, self.nodes[b].left);
            self.pull(b);
            b
        }
    }

    fn split(&mut self, root: usize, count: usize) -> (usize, usize) {
        if root == NONE {
            return (NONE, NONE);
        }
        let left = self.count(self.nodes[root].left);
        let (a, b) = if count <= left {
            let (a, b) = self.split(self.nodes[root].left, count);
            self.nodes[root].left = b;
            self.pull(root);
            (a, root)
        } else {
            let (a, b) = self.split(self.nodes[root].right, count - left - 1);
            self.nodes[root].right = a;
            self.pull(root);
            (root, b)
        };
        if a != NONE {
            self.nodes[a].parent = NONE;
        }
        if b != NONE {
            self.nodes[b].parent = NONE;
        }
        (a, b)
    }

    fn rank(&self, mut i: usize) -> usize {
        let mut rank = self.count(self.nodes[i].left);
        while self.nodes[i].parent != NONE {
            let p = self.nodes[i].parent;
            if self.nodes[p].right == i {
                rank += self.count(self.nodes[p].left) + 1;
            }
            i = p;
        }
        rank
    }

    fn schedule(&mut self, a: usize, y: i64) {
        if a == NONE {
            return;
        }
        let b = self.nodes[a].next;
        let key = if b == NONE {
            None
        } else {
            self.nodes[a]
                .line
                .crossing(self.nodes[b].line)
                .filter(|&at| at >= y && at < self.nodes[a].bottom.min(self.nodes[b].bottom))
        };
        self.events.set(a, key);
    }

    fn insert(&mut self, i: usize, y: i64) {
        let (mut cursor, mut rank, mut previous, mut next) = (self.root, 0, NONE, NONE);
        while cursor != NONE {
            if self.nodes[i]
                .line
                .compare(self.nodes[cursor].line, y)
                .is_lt()
            {
                next = cursor;
                cursor = self.nodes[cursor].left;
            } else {
                previous = cursor;
                rank += 1 + self.count(self.nodes[cursor].left);
                cursor = self.nodes[cursor].right;
            }
        }
        self.nodes[i].previous = previous;
        self.nodes[i].next = next;
        if previous != NONE {
            self.nodes[previous].next = i;
        }
        if next != NONE {
            self.nodes[next].previous = i;
        }
        let (a, b) = self.split(self.root, rank);
        let a = self.merge(a, i);
        self.root = self.merge(a, b);
        self.nodes[self.root].parent = NONE;
        self.schedule(previous, y);
        self.schedule(i, y);
    }

    fn visibility(&mut self, i: usize, sign: i32, y: i64) {
        let node = &mut self.nodes[i];
        if sign == node.sign {
            return;
        }
        if node.sign != 0 && node.opened < y {
            self.output.push(Visible {
                source: node.source,
                top: node.opened,
                bottom: y,
                sign: node.sign,
            });
        }
        node.sign = sign;
        node.opened = y;
    }

    fn remove(&mut self, i: usize, y: i64) {
        self.visibility(i, 0, y);
        let (previous, next) = (self.nodes[i].previous, self.nodes[i].next);
        if previous != NONE {
            self.nodes[previous].next = next;
        }
        if next != NONE {
            self.nodes[next].previous = previous;
        }
        self.events.set(i, None);
        let rank = self.rank(i);
        let (a, b) = self.split(self.root, rank);
        let (_, b) = self.split(b, 1);
        self.root = self.merge(a, b);
        if self.root != NONE {
            self.nodes[self.root].parent = NONE;
        }
        self.schedule(previous, y);
    }

    fn exchange(&mut self, a: usize, b: usize, y: i64) {
        let (previous, next) = (self.nodes[a].previous, self.nodes[b].next);
        let rank = self.rank(a);
        let (left, rest) = self.split(self.root, rank);
        let (pair, right) = self.split(rest, 2);
        let (a, b) = self.split(pair, 1);
        let middle = self.merge(b, a);
        let left = self.merge(left, middle);
        self.root = self.merge(left, right);
        self.nodes[self.root].parent = NONE;
        self.nodes[b].previous = previous;
        self.nodes[b].next = a;
        self.nodes[a].previous = b;
        self.nodes[a].next = next;
        if previous != NONE {
            self.nodes[previous].next = b;
        }
        if next != NONE {
            self.nodes[next].previous = a;
        }
        self.schedule(previous, y);
        self.schedule(b, y);
        self.schedule(a, y);
    }

    fn refresh(&mut self, i: usize, incoming: i32, y: i64, rule: FillRule) {
        if i == NONE {
            return;
        }
        let node = &self.nodes[i];
        if !node.dirty && node.incoming == incoming {
            return;
        }
        if node.visible == 0
            && rule == FillRule::NonZero
            && (incoming + node.min > 0 || incoming + node.max < 0)
        {
            self.nodes[i].incoming = incoming;
            self.nodes[i].dirty = false;
            return;
        }
        let (left, right) = (node.left, node.right);
        let before = incoming + self.sum(left);
        let after = before + self.nodes[i].weight;
        self.refresh(left, incoming, y, rule);
        let inside = |w| match rule {
            FillRule::NonZero => w != 0,
            FillRule::EvenOdd => w & 1 != 0,
        };
        self.visibility(i, i32::from(inside(after)) - i32::from(inside(before)), y);
        self.refresh(right, after, y, rule);
        let visible = usize::from(self.nodes[i].sign != 0)
            + if left == NONE {
                0
            } else {
                self.nodes[left].visible
            }
            + if right == NONE {
                0
            } else {
                self.nodes[right].visible
            };
        self.nodes[i].visible = visible;
        self.nodes[i].incoming = incoming;
        self.nodes[i].dirty = false;
    }
}
