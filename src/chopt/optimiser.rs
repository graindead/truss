// Ported from CHOpt (https://github.com/GenericMadScientist/CHOpt)
// Copyright (C) 2020-2026 Raymond Wright. Rust port for Truss.
// Licensed under the GNU General Public License v3.0 or later.

//! Finds the highest-scoring Star Power path.
//!
//! Vertices are states (the next point that matters and where Star Power can start from),
//! edges are activations weighted by the points they add. The best path is the heaviest path
//! through this graph, found depth first with memoised subpath values.

use super::processed::{ActValidity, Activation, ActivationCandidate, Path, ProcessedSong, ProtoActivation};
use super::sp::SpBar;
use super::time::SpPosition;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy)]
struct Vertex {
    point: usize,
    position: SpPosition,
    is_max_sp_vertex: bool,
}

/// Hashable form of a vertex (doubles compared by value, with -0 == 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VertexKey(usize, u64, u64, bool);

fn key_bits(x: f64) -> u64 {
    if x == 0.0 { 0 } else { x.to_bits() }
}

impl Vertex {
    fn key(&self) -> VertexKey {
        VertexKey(self.point, key_bits(self.position.beat), key_bits(self.position.sp_measure), self.is_max_sp_vertex)
    }
}

/// Reusable arrays for `ActivationEndSet` (stamped, so nothing needs clearing).
pub struct EndScratch {
    next: Vec<u32>,
    stamp: Vec<u32>,
    temp: Vec<u32>,
    generation: u32,
    temp_generation: u32,
}

impl EndScratch {
    fn new(n: usize) -> Self {
        EndScratch { next: vec![0; n + 2], stamp: vec![0; n + 2], temp: vec![0; n + 2], generation: 0, temp_generation: 0 }
    }
}

/// Activation ends already reached: a "next unreached" chain over the permanent ones (which
/// behaves like CHOpt's [start, min_absent) range plus extra elements) and temporary ones that
/// stop counting once another phrase is gained.
struct ActivationEndSet<'s> {
    start: usize,
    end: usize,
    s: &'s mut EndScratch,
}

impl<'s> ActivationEndSet<'s> {
    fn new(start: usize, end: usize, s: &'s mut EndScratch) -> Self {
        s.generation += 1;
        s.temp_generation += 1;
        ActivationEndSet { start, end, s }
    }
    fn reset(&mut self, start: usize) {
        self.start = start;
        self.s.generation += 1;
        self.s.temp_generation += 1;
    }
    fn link(&self, x: usize) -> Option<usize> {
        (self.s.stamp[x] == self.s.generation).then(|| self.s.next[x] as usize)
    }
    fn is_temporary(&self, x: usize) -> bool {
        self.s.temp[x] == self.s.temp_generation
    }
    /// The first element at or after `x` that hasn't been permanently reached.
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while let Some(n) = self.link(root) {
            root = n;
        }
        let mut cur = x;
        while cur != root {
            let n = self.s.next[cur] as usize;
            self.s.next[cur] = root as u32;
            cur = n;
        }
        root
    }
    /// Like CHOpt, this only skips temporary elements after the reached range.
    fn lowest_absent_element(&mut self) -> usize {
        let mut x = self.find(self.start);
        while x < self.end && self.is_temporary(x) {
            x += 1;
        }
        x.min(self.end)
    }
    fn next_absent_element(&mut self, element: usize) -> usize {
        let min = self.find(self.start);
        let mut x = self.find((element + 1).max(min));
        while x < self.end && self.is_temporary(x) {
            x = self.find(x + 1);
        }
        x.min(self.end)
    }
    fn add(&mut self, element: usize) {
        self.s.stamp[element] = self.s.generation;
        self.s.next[element] = element as u32 + 1;
    }
    fn add_temporary_element(&mut self, element: usize) {
        self.s.temp[element] = self.s.temp_generation;
    }
    fn clear_temporary_elements(&mut self) {
        self.s.temp_generation += 1;
    }
}

struct AggEdge {
    dest: Vertex,
    weight: i32,
    acts: Vec<ProtoActivation>,
}

/// Out edges of one vertex, keeping only the heaviest edge to each destination.
#[derive(Default)]
struct OutEdgeAggregate {
    edges: Vec<AggEdge>,
    index: HashMap<VertexKey, usize>,
}

impl OutEdgeAggregate {
    fn add_activation(&mut self, dest: Vertex, act: Option<ProtoActivation>, weight: i32) {
        match self.index.get(&dest.key()) {
            None => {
                self.index.insert(dest.key(), self.edges.len());
                self.edges.push(AggEdge { dest, weight, acts: act.into_iter().collect() });
            }
            Some(&i) => {
                let e = &mut self.edges[i];
                if e.weight > weight {
                    return;
                }
                if e.weight < weight {
                    e.acts.clear();
                    e.weight = weight;
                }
                e.acts.extend(act);
            }
        }
    }
}

struct Edge {
    dest: usize,
    weight: i32,
    acts: Vec<ProtoActivation>,
}

struct PathGraph {
    adjacency: Vec<Vec<Edge>>,
    vertices: Vec<Vertex>,
    lookup: HashMap<VertexKey, usize>,
    optimal: Vec<Option<i32>>,
}

impl PathGraph {
    fn new(root: Vertex) -> Self {
        let mut lookup = HashMap::new();
        lookup.insert(root.key(), 0);
        PathGraph { adjacency: vec![Vec::new()], vertices: vec![root], lookup, optimal: vec![None] }
    }
    /// The vertex's id, and whether it is new.
    fn insert_vertex(&mut self, v: Vertex) -> (usize, bool) {
        if let Some(&id) = self.lookup.get(&v.key()) {
            return (id, false);
        }
        let id = self.vertices.len();
        self.lookup.insert(v.key(), id);
        self.vertices.push(v);
        self.adjacency.push(Vec::new());
        self.optimal.push(None);
        (id, true)
    }

    /// Vertices in an order where every vertex comes after all those its edges lead to.
    fn post_order(&self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.vertices.len());
        let mut seen = vec![false; self.vertices.len()];
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        seen[0] = true;
        while let Some(top) = stack.last_mut() {
            let (id, next) = *top;
            if let Some(e) = self.adjacency[id].get(next) {
                top.1 += 1;
                let dest = e.dest;
                if !seen[dest] {
                    seen[dest] = true;
                    stack.push((dest, 0));
                }
            } else {
                order.push(id);
                stack.pop();
            }
        }
        order
    }
    fn prune_suboptimal_out_edges(&mut self, id: usize) {
        let optimal = &self.optimal;
        let weight = |e: &Edge| e.weight + optimal[e.dest].unwrap_or(0);
        let edges = &mut self.adjacency[id];
        let best = edges.iter().map(weight).max().unwrap_or(0);
        // Same element order as std::partition, so ties pick the same path as CHOpt.
        let is_best = |e: &Edge| weight(e) == best;
        let (mut first, mut last) = (0, edges.len());
        loop {
            while first < last && is_best(&edges[first]) {
                first += 1;
            }
            if first == last {
                break;
            }
            last -= 1;
            while first < last && !is_best(&edges[last]) {
                last -= 1;
            }
            if first == last {
                break;
            }
            edges.swap(first, last);
            first += 1;
        }
        edges.truncate(first);
        self.optimal[id] = Some(best);
    }
}

pub struct Optimiser<'a> {
    song: &'a ProcessedSong,
    cancel: &'a AtomicBool,
    whammy_delay: f64,
    next_candidate_points: Vec<usize>,
}

/// Threads for the search: half the cores (at least one), so games, OBS and the rest of Truss
/// keep their share of the CPU.
fn worker_count() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

/// Lowers the current thread's priority, so the search gives way to everything else.
fn lower_priority() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

impl<'a> Optimiser<'a> {
    pub fn new(song: &'a ProcessedSong, cancel: &'a AtomicBool, whammy_delay: f64) -> Self {
        let pts = &song.points.points;
        let n = pts.len();
        let mut next = vec![n; n + 1];
        let mut cand = n;
        for i in (0..n).rev() {
            let p = &pts[i];
            if p.is_sp_granting_note || (p.is_hold_point && song.sp_data.is_in_whammy_ranges(p.position.beat)) {
                cand = i;
            }
            next[i] = cand;
        }
        Optimiser { song, cancel, whammy_delay, next_candidate_points: next }
    }

    fn n(&self) -> usize {
        self.song.points.len()
    }

    /// The best path, or `None` if cancelled.
    pub fn optimal_path(&self) -> Option<Path> {
        let root = self.advance_graph_vertex(Vertex { point: 0, position: SpPosition::NEG_INF, is_max_sp_vertex: false });
        let graph = self.path_graph(root)?;
        Some(self.optimal_path_from_graph(&graph))
    }

    /// Builds the graph a wave at a time (each wave's vertices expanded in parallel), then
    /// keeps only the best edges, deepest vertices first. Each vertex's edges are the same,
    /// in the same order, as when built one at a time, so the result is too.
    fn path_graph(&self, root: Vertex) -> Option<PathGraph> {
        let mut graph = PathGraph::new(root);
        let threads = worker_count();
        let mut scratches: Vec<EndScratch> = (0..threads).map(|_| EndScratch::new(self.n())).collect();
        let mut frontier = vec![0usize];
        while !frontier.is_empty() {
            let vertices: Vec<Vertex> = frontier.iter().map(|&id| graph.vertices[id]).collect();
            let results = self.expand(&vertices, &mut scratches)?;
            let mut next = Vec::new();
            for (&id, agg) in frontier.iter().zip(results) {
                for e in agg.edges {
                    let (dest, new) = graph.insert_vertex(e.dest);
                    if new {
                        next.push(dest);
                    }
                    graph.adjacency[id].push(Edge { dest, weight: e.weight, acts: e.acts });
                }
            }
            frontier = next;
        }
        for id in graph.post_order() {
            graph.prune_suboptimal_out_edges(id);
        }
        Some(graph)
    }

    /// The out edges of each vertex, worked out on up to `scratches.len()` threads.
    fn expand(&self, vertices: &[Vertex], scratches: &mut [EndScratch]) -> Option<Vec<OutEdgeAggregate>> {
        if scratches.len() == 1 || vertices.len() == 1 {
            let mut out = Vec::with_capacity(vertices.len());
            for v in vertices {
                if self.cancel.load(Ordering::Relaxed) {
                    return None;
                }
                out.push(self.out_edges(*v, &mut scratches[0]));
            }
            return Some(out);
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut done: Vec<(usize, OutEdgeAggregate)> = std::thread::scope(|s| {
            let handles: Vec<_> = scratches
                .iter_mut()
                .take(vertices.len())
                .map(|scratch| {
                    let next = &next;
                    s.spawn(move || {
                        lower_priority();
                        let mut mine = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            if i >= vertices.len() || self.cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            mine.push((i, self.out_edges(vertices[i], scratch)));
                        }
                        mine
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        if self.cancel.load(Ordering::Relaxed) || done.len() != vertices.len() {
            return None;
        }
        done.sort_unstable_by_key(|(i, _)| *i);
        Some(done.into_iter().map(|(_, agg)| agg).collect())
    }

    fn advance_graph_vertex(&self, mut v: Vertex) -> Vertex {
        v.point = self.next_candidate_points[v.point.min(self.n())];
        if v.point == self.n() {
            v.position = SpPosition::POS_INF;
            return v;
        }
        v = self.add_whammy_delay(v);
        let point = v.point.saturating_sub(1);
        let pos = self.song.points.get(point).max_sqz_hit_window_start;
        if pos.beat >= v.position.beat {
            v.position = pos;
        }
        v
    }

    fn add_whammy_delay(&self, mut v: Vertex) -> Vertex {
        let tm = &self.song.time_map;
        let secs = tm.beats_to_seconds(v.position.beat) + self.whammy_delay;
        v.position = tm.pos(tm.seconds_to_beats(secs));
        v
    }

    fn out_edges(&self, v: Vertex, scratch: &mut EndScratch) -> OutEdgeAggregate {
        let song = self.song;
        let n = self.n();
        let mut attained = ActivationEndSet::new(v.point, n, scratch);
        let mut lower_bound_set = false;
        let mut out = OutEdgeAggregate::default();
        for p in v.point..n {
            let mut bar = SpBar::new(1.0, 1.0, song.values);
            let mut starting_pos = SpPosition::NEG_INF;
            if p != 0 {
                starting_pos = song.points.get(p - 1).hit_window_start;
            }
            if !v.is_max_sp_vertex {
                let (b, pos) = song.total_available_sp_with_earliest_pos(v.position.beat, v.point, p, starting_pos);
                bar = b;
                starting_pos = pos;
            }
            if !bar.full_enough_to_activate() {
                continue;
            }
            if p != v.point && bar.min == 1.0 && song.points.get(p - 1).is_sp_granting_note {
                let full = Vertex { point: p, position: song.points.get(p - 1).hit_window_start, is_max_sp_vertex: true };
                out.add_activation(full, None, 0);
                break;
            }
            // Skip ends too early for even the earliest activation.
            if !lower_bound_set {
                let act_length = 8.0 * bar.min.max(song.values.minimum_to_activate);
                let earliest_act_end = starting_pos.sp_measure + act_length;
                let mut q = p + 1;
                while q < n && song.points.get(q).hit_window_end.sp_measure <= earliest_act_end {
                    q += 1;
                }
                attained.reset(q - 1);
                lower_bound_set = true;
            }
            self.add_acts_from_starting_point(p, starting_pos, bar, &mut attained, &mut out);
            if song.points.get(p).is_sp_granting_note {
                attained.clear_temporary_elements();
            }
        }
        out
    }

    fn add_acts_from_starting_point(&self, start: usize, starting_pos: SpPosition, bar: SpBar, attained: &mut ActivationEndSet<'_>, out: &mut OutEdgeAggregate) {
        let song = self.song;
        let n = self.n();
        let mut q = attained.lowest_absent_element();
        while q < n {
            let cand = ActivationCandidate { act_start: start, act_end: q, earliest_activation_point: starting_pos, sp_bar: bar };
            let result = song.is_candidate_valid(&cand, 1.0, SpPosition::NEG_INF);
            match result.validity {
                ActValidity::InsufficientSp => {
                    if !song.points.get(q).is_hold_point {
                        return;
                    }
                    q = song.points.next_non_hold_point(q) - 1;
                    q = attained.next_absent_element(q);
                    continue;
                }
                ActValidity::SurplusSp => {
                    if bar.minimum_sufficient_to_activate() || !self.act_contains_sp_phrase(start, q) {
                        attained.add(q);
                    } else {
                        attained.add_temporary_element(q);
                    }
                    q = attained.next_absent_element(q);
                    continue;
                }
                ActValidity::Success => attained.add(q),
            }
            let score = song.points.range_score(start, q + 1);
            let dest = self.advance_graph_vertex(Vertex {
                point: song.points.first_after_current_phrase(q),
                position: result.ending_position,
                is_max_sp_vertex: false,
            });
            out.add_activation(dest, Some(ProtoActivation { act_start: start, act_end: q }), score);
            q = attained.next_absent_element(q);
        }
    }

    fn act_contains_sp_phrase(&self, start: usize, end: usize) -> bool {
        self.song.points.next_sp_granting_note(start) <= end
    }

    fn optimal_path_from_graph(&self, graph: &PathGraph) -> Path {
        let mut path = Path::default();
        let mut src = 0;
        let mut act_start_vertex = graph.vertices[0];
        loop {
            let Some(edge) = graph.adjacency[src].first() else { break };
            path.score_boost += edge.weight;
            if edge.acts.is_empty() {
                src = edge.dest;
                continue;
            }
            let mut best = edge.acts[0];
            let mut best_sqz = self.act_squeeze_level(best, act_start_vertex);
            for &a in &edge.acts[1..] {
                let s = self.act_squeeze_level(a, act_start_vertex);
                if s < best_sqz {
                    best = a;
                    best_sqz = s;
                }
            }
            let min_whammy_force = self.forced_whammy_end(best, act_start_vertex, best_sqz);
            let (sp_start, sp_end) = self.act_duration(best, act_start_vertex, best_sqz, min_whammy_force);
            let mut act = Activation { act_start: best.act_start, act_end: best.act_end, whammy_end: min_whammy_force.beat, sp_start, sp_end };
            let end_vertex = graph.vertices[edge.dest];
            if end_vertex.point != self.n() {
                act.sp_end = act.sp_end.min(self.song.sp_data.next_whammy_point(end_vertex.position.beat));
            }
            path.activations.push(act);
            src = edge.dest;
            act_start_vertex = end_vertex;
        }
        path
    }

    fn act_squeeze_level(&self, act: ProtoActivation, v: Vertex) -> f64 {
        let song = self.song;
        let (mut min_sqz, mut max_sqz) = (0.0, 1.0);
        let bound = act.act_start.saturating_sub(1);
        while max_sqz - min_sqz > 0.01 {
            let trial = (min_sqz + max_sqz) / 2.0;
            let mut start_pos = song.adjusted_hit_window_start(bound, trial);
            if start_pos.beat < v.position.beat {
                start_pos = v.position;
            }
            let (bar, new_pos) = song.total_available_sp_with_earliest_pos(v.position.beat, v.point, act.act_start, start_pos);
            let cand = ActivationCandidate { act_start: act.act_start, act_end: act.act_end, earliest_activation_point: new_pos, sp_bar: bar };
            if song.is_candidate_valid(&cand, trial, SpPosition::NEG_INF).validity == ActValidity::Success {
                max_sqz = trial;
            } else {
                min_sqz = trial;
            }
        }
        max_sqz
    }

    fn forced_whammy_end(&self, act: ProtoActivation, v: Vertex, sqz: f64) -> SpPosition {
        let song = self.song;
        let next = act.act_end + 1;
        if next == self.n() {
            return SpPosition::POS_INF;
        }
        let prev = act.act_start.saturating_sub(1);
        let mut min_force = v.position;
        let mut max_force = song.points.get(next).hit_window_end;
        let start_pos = song.adjusted_hit_window_start(prev, sqz);
        while max_force.beat - min_force.beat > 0.01 {
            let mid = song.time_map.pos((min_force.beat + max_force.beat) * 0.5);
            let bar = song.total_available_sp(v.position.beat, v.point, act.act_start, mid.beat);
            let cand = ActivationCandidate { act_start: act.act_start, act_end: act.act_end, earliest_activation_point: start_pos, sp_bar: bar };
            if song.is_candidate_valid(&cand, sqz, mid).validity == ActValidity::Success {
                min_force = mid;
            } else {
                max_force = mid;
            }
        }
        min_force
    }

    fn act_duration(&self, act: ProtoActivation, v: Vertex, sqz: f64, min_whammy_force: SpPosition) -> (f64, f64) {
        let song = self.song;
        let bound = act.act_start.saturating_sub(1);
        let mut min_pos = song.adjusted_hit_window_start(bound, sqz);
        let mut max_pos = song.adjusted_hit_window_end(act.act_start, sqz);
        let bar = song.total_available_sp(v.position.beat, v.point, act.act_start, min_whammy_force.beat);
        while max_pos.beat - min_pos.beat > 0.01 {
            let trial = song.time_map.pos((min_pos.beat + max_pos.beat) * 0.5);
            let cand = ActivationCandidate { act_start: act.act_start, act_end: act.act_end, earliest_activation_point: trial, sp_bar: bar };
            if song.is_candidate_valid(&cand, sqz, min_whammy_force).validity == ActValidity::Success {
                min_pos = trial;
            } else {
                max_pos = trial;
            }
        }
        let cand = ActivationCandidate { act_start: act.act_start, act_end: act.act_end, earliest_activation_point: min_pos, sp_bar: bar };
        let result = song.is_candidate_valid(&cand, sqz, min_whammy_force);
        (min_pos.beat, result.ending_position.beat)
    }
}
