//! Bounded k-shortest simple paths (Yen's algorithm) with an A* speedup.
//!
//! The stock `pathfinding::yen` runs a blind Dijkstra for every spur search,
//! which dominates runtime on large graphs. Instead we precompute, once, the
//! shortest distance from every node to the target (a reverse Dijkstra) and use
//! it as an admissible, consistent A* heuristic for every spur search - so each
//! search heads straight at the target rather than exploring outward. A
//! wall-clock deadline bounds the whole thing, mirroring the Python engine, so
//! it degrades gracefully on huge graphs instead of running away.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::time::Instant;

type Adj = Vec<Vec<(usize, u64)>>;

/// Shortest distance from every node to `target` (Dijkstra on the reversed graph).
fn distances_to_target(adj: &Adj, target: usize) -> Vec<u64> {
    let n = adj.len();
    let mut radj: Adj = vec![Vec::new(); n];
    for (u, succ) in adj.iter().enumerate() {
        for &(v, w) in succ {
            radj[v].push((u, w));
        }
    }
    let mut dist = vec![u64::MAX; n];
    let mut heap = BinaryHeap::new();
    dist[target] = 0;
    heap.push(Reverse((0u64, target)));
    while let Some(Reverse((d, u))) = heap.pop() {
        if d > dist[u] {
            continue;
        }
        for &(v, w) in &radj[u] {
            let nd = d.saturating_add(w);
            if nd < dist[v] {
                dist[v] = nd;
                heap.push(Reverse((nd, v)));
            }
        }
    }
    dist
}

/// A* shortest path from `src` to `tgt`, skipping removed edges and nodes, using
/// `h` (distance-to-target) as the heuristic. Returns the node path if found.
fn astar(
    adj: &Adj,
    src: usize,
    tgt: usize,
    h: &[u64],
    removed_edges: &HashSet<(usize, usize)>,
    removed_nodes: &HashSet<usize>,
) -> Option<Vec<usize>> {
    if removed_nodes.contains(&src) || h[src] == u64::MAX {
        return None;
    }
    let n = adj.len();
    let mut g = vec![u64::MAX; n];
    let mut prev = vec![usize::MAX; n];
    let mut closed = vec![false; n];
    let mut heap = BinaryHeap::new();
    g[src] = 0;
    heap.push(Reverse((h[src], src)));
    while let Some(Reverse((_f, u))) = heap.pop() {
        if closed[u] {
            continue;
        }
        closed[u] = true;
        if u == tgt {
            let mut path = vec![tgt];
            let mut c = tgt;
            while c != src {
                c = prev[c];
                path.push(c);
            }
            path.reverse();
            return Some(path);
        }
        for &(v, w) in &adj[u] {
            if closed[v] || removed_nodes.contains(&v) || h[v] == u64::MAX {
                continue;
            }
            if removed_edges.contains(&(u, v)) {
                continue;
            }
            let ng = g[u].saturating_add(w);
            if ng < g[v] {
                g[v] = ng;
                prev[v] = u;
                heap.push(Reverse((ng.saturating_add(h[v]), v)));
            }
        }
    }
    None
}

fn path_weight(adj: &Adj, path: &[usize]) -> u64 {
    let mut total = 0u64;
    for w in path.windows(2) {
        if let Some(&(_, cost)) = adj[w[0]].iter().find(|&&(v, _)| v == w[1]) {
            total = total.saturating_add(cost);
        }
    }
    total
}

/// Up to `k` shortest simple paths from `src` to `tgt`, bounded by `deadline`.
pub fn yen_bounded(
    adj: &Adj,
    src: usize,
    tgt: usize,
    k: usize,
    deadline: Instant,
) -> Vec<Vec<usize>> {
    let h = distances_to_target(adj, tgt);
    let no_e = HashSet::new();
    let no_n = HashSet::new();
    let first = match astar(adj, src, tgt, &h, &no_e, &no_n) {
        Some(p) => p,
        None => return Vec::new(),
    };

    let mut a: Vec<Vec<usize>> = vec![first];
    let mut b: BinaryHeap<Reverse<(u64, Vec<usize>)>> = BinaryHeap::new();
    let mut seen: HashSet<Vec<usize>> = HashSet::new();
    seen.insert(a[0].clone());

    while a.len() < k {
        if Instant::now() >= deadline {
            break;
        }
        let prev = a.last().unwrap().clone();
        for i in 0..prev.len().saturating_sub(1) {
            if Instant::now() >= deadline {
                break;
            }
            let spur = prev[i];
            let root = &prev[0..=i];

            // Remove the edges taken by earlier paths sharing this root.
            let mut removed_edges: HashSet<(usize, usize)> = HashSet::new();
            for p in &a {
                if p.len() > i && &p[0..=i] == root {
                    removed_edges.insert((p[i], p[i + 1]));
                }
            }
            // Keep the spur path node-disjoint from the root (except the spur).
            let removed_nodes: HashSet<usize> = root[0..i].iter().copied().collect();

            if let Some(spur_path) = astar(adj, spur, tgt, &h, &removed_edges, &removed_nodes) {
                let mut total: Vec<usize> = root[0..i].to_vec();
                total.extend(spur_path);
                if !seen.contains(&total) {
                    let cost = path_weight(adj, &total);
                    seen.insert(total.clone());
                    b.push(Reverse((cost, total)));
                }
            }
        }
        match b.pop() {
            Some(Reverse((_c, path))) => a.push(path),
            None => break,
        }
    }
    a
}
