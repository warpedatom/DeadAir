//! Noise-weighted path solving: threshold sweep + Yen's k-shortest, re-ranked
//! by the max/mean path score, with a detection-probability model, a Pareto
//! mode, and node/edge-type constraints. A faithful port of NoiseHound's Python
//! solver so results match.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use pathfinding::prelude::{bfs, dijkstra};

use crate::kshortest::yen_bounded;
use crate::model::ScoredGraph;

pub struct Config {
    pub max_weight: f64,
    pub mean_weight: f64,
    pub correlation: f64,
    pub candidates: usize,
    pub time_budget_s: f64,
}

#[derive(Clone)]
pub struct EdgeInfo {
    pub edge_type: String,
    pub noise: f64,
    pub known: bool,
}

pub struct Graph {
    pub ids: Vec<String>,
    pub names: Vec<String>,
    pub node_types: Vec<String>,
    pub adj: Vec<Vec<(usize, u64)>>,
    pub edges: HashMap<(usize, usize), EdgeInfo>,
    id_to_idx: HashMap<String, usize>,
    name_to_idx: HashMap<String, Vec<usize>>,
}

/// Integer weight for path enumeration (noise is <=2 decimal places in practice).
fn weight_of(noise: f64) -> u64 {
    (noise * 100.0).round().max(0.0) as u64
}

impl Graph {
    /// Build a graph, dropping avoided nodes and edge types (source/objective
    /// are always kept even if listed in `avoid_nodes`).
    pub fn build(
        sg: &ScoredGraph,
        avoid_nodes: &HashSet<String>,
        avoid_edges: &HashSet<String>,
        keep_ids: &HashSet<String>,
    ) -> Graph {
        let avoid_edges_lc: HashSet<String> =
            avoid_edges.iter().map(|s| s.to_lowercase()).collect();

        let mut id_to_idx = HashMap::new();
        let mut ids = Vec::new();
        let mut names = Vec::new();
        let mut node_types = Vec::new();
        let mut name_to_idx: HashMap<String, Vec<usize>> = HashMap::new();

        for n in &sg.nodes {
            if avoid_nodes.contains(&n.id) && !keep_ids.contains(&n.id) {
                continue;
            }
            let idx = names.len();
            id_to_idx.insert(n.id.clone(), idx);
            ids.push(n.id.clone());
            let display = n
                .name
                .clone()
                .unwrap_or_else(|| n.id.clone())
                .to_uppercase();
            name_to_idx.entry(display.clone()).or_default().push(idx);
            names.push(display);
            node_types.push(n.node_type.clone().unwrap_or_else(|| "Base".into()));
        }

        let mut adj: Vec<Vec<(usize, u64)>> = vec![Vec::new(); names.len()];
        let mut edges = HashMap::new();
        for e in &sg.edges {
            if avoid_edges_lc.contains(&e.edge_type.to_lowercase()) {
                continue;
            }
            let (u, v) = match (id_to_idx.get(&e.source), id_to_idx.get(&e.target)) {
                (Some(&u), Some(&v)) => (u, v),
                _ => continue,
            };
            // Keep the quietest edge per ordered pair (matches annotate's collapse).
            let replace = edges
                .get(&(u, v))
                .map(|ei: &EdgeInfo| e.noise < ei.noise)
                .unwrap_or(true);
            if replace {
                edges.insert(
                    (u, v),
                    EdgeInfo {
                        edge_type: e.edge_type.clone(),
                        noise: e.noise,
                        known: e.corpus_known,
                    },
                );
            }
            adj[u].retain(|&(t, _)| t != v);
            adj[u].push((v, weight_of(edges[&(u, v)].noise)));
        }

        Graph {
            ids,
            names,
            node_types,
            adj,
            edges,
            id_to_idx,
            name_to_idx,
        }
    }

    /// Resolve an identifier: object id, then exact name, then name without the
    /// @DOMAIN suffix (preferring a Group on a tie).
    pub fn resolve(&self, ident: &str) -> Option<usize> {
        if let Some(&i) = self.id_to_idx.get(ident) {
            return Some(i);
        }
        let want = ident.trim().to_uppercase();
        if let Some(v) = self.name_to_idx.get(&want) {
            return Some(self.prefer_group(v));
        }
        let stripped: Vec<usize> = self
            .name_to_idx
            .iter()
            .filter(|(name, _)| name.split('@').next().unwrap_or("") == want)
            .flat_map(|(_, v)| v.clone())
            .collect();
        if stripped.is_empty() {
            None
        } else {
            Some(self.prefer_group(&stripped))
        }
    }

    fn prefer_group(&self, idxs: &[usize]) -> usize {
        idxs.iter()
            .find(|&&i| self.node_types[i] == "Group")
            .copied()
            .unwrap_or(idxs[0])
    }

    fn scores_along(&self, path: &[usize]) -> Vec<f64> {
        path.windows(2)
            .map(|w| self.edges[&(w[0], w[1])].noise)
            .collect()
    }
}

fn score_path(scores: &[f64], cfg: &Config) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    let loudest = scores.iter().cloned().fold(f64::MIN, f64::max);
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    loudest * cfg.max_weight + mean * cfg.mean_weight
}

fn detection_probability(scores: &[f64], cfg: &Config) -> f64 {
    if scores.is_empty() {
        return 0.0;
    }
    let probs: Vec<f64> = scores.iter().map(|s| (s / 100.0).clamp(0.0, 1.0)).collect();
    let noisy_or = 1.0 - probs.iter().map(|p| 1.0 - p).product::<f64>();
    let loudest = probs.iter().cloned().fold(f64::MIN, f64::max);
    cfg.correlation * loudest + (1.0 - cfg.correlation) * noisy_or
}

pub struct Scored {
    pub path_score: f64,
    pub prob: f64,
    pub hops: usize,
    pub sum_noise: f64,
    pub nodes: Vec<usize>,
}

fn candidate_paths(g: &Graph, src: usize, dst: usize, cfg: &Config) -> Vec<Vec<usize>> {
    let mut seen: HashSet<Vec<usize>> = HashSet::new();
    let mut out: Vec<Vec<usize>> = Vec::new();

    let push = |seen: &mut HashSet<Vec<usize>>, out: &mut Vec<Vec<usize>>, p: Vec<usize>| {
        if seen.insert(p.clone()) {
            out.push(p);
        }
    };

    // (1) Threshold sweep: for each distinct edge weight, the min-weight and
    // min-hop route staying under it. The correctness backstop.
    let thresholds: BTreeSet<u64> = g
        .adj
        .iter()
        .flat_map(|v| v.iter().map(|&(_, w)| w))
        .collect();
    for &t in &thresholds {
        if let Some((path, _)) = dijkstra(
            &src,
            |&n| {
                g.adj[n]
                    .iter()
                    .filter(|&&(_, w)| w <= t)
                    .map(|&(m, w)| (m, w))
            },
            |&n| n == dst,
        ) {
            push(&mut seen, &mut out, path);
        }
        if let Some(path) = bfs(
            &src,
            |&n| {
                g.adj[n]
                    .iter()
                    .filter(|&&(_, w)| w <= t)
                    .map(|&(m, _)| m)
                    .collect::<Vec<_>>()
            },
            |&n| n == dst,
        ) {
            push(&mut seen, &mut out, path);
        }
    }

    // (2) Yen's k-shortest by weight - breadth. A*-accelerated and bounded by a
    // wall-clock budget so a huge graph degrades gracefully (the threshold sweep
    // above already guarantees a strong answer).
    let deadline = Instant::now() + Duration::from_secs_f64(cfg.time_budget_s.max(0.0));
    for path in yen_bounded(&g.adj, src, dst, cfg.candidates.max(1), deadline) {
        push(&mut seen, &mut out, path);
    }

    out
}

fn score_all(g: &Graph, candidates: Vec<Vec<usize>>, cfg: &Config) -> Vec<Scored> {
    candidates
        .into_iter()
        .map(|nodes| {
            let scores = g.scores_along(&nodes);
            Scored {
                path_score: score_path(&scores, cfg),
                prob: detection_probability(&scores, cfg),
                hops: nodes.len() - 1,
                sum_noise: scores.iter().sum(),
                nodes,
            }
        })
        .collect()
}

fn dominates(b: &Scored, a: &Scored) -> bool {
    let ge = b.path_score <= a.path_score && b.hops <= a.hops && b.prob <= a.prob;
    let strict = b.path_score < a.path_score || b.hops < a.hops || b.prob < a.prob;
    ge && strict
}

/// Solve. `mode` is "noise", "probability", or "pareto".
pub fn solve(g: &Graph, src: usize, dst: usize, k: usize, mode: &str, cfg: &Config) -> Vec<Scored> {
    if src == dst {
        return Vec::new();
    }
    let mut scored = score_all(g, candidate_paths(g, src, dst, cfg), cfg);
    if scored.is_empty() {
        return Vec::new();
    }

    match mode {
        "pareto" => {
            let front: Vec<Scored> = {
                let mut keep = Vec::new();
                for i in 0..scored.len() {
                    if !scored
                        .iter()
                        .enumerate()
                        .any(|(j, b)| j != i && dominates(b, &scored[i]))
                    {
                        keep.push(i);
                    }
                }
                let keepset: HashSet<usize> = keep.into_iter().collect();
                let mut v = Vec::new();
                for (i, s) in scored.drain(..).enumerate() {
                    if keepset.contains(&i) {
                        v.push(s);
                    }
                }
                v
            };
            let mut front = front;
            front.sort_by(|a, b| {
                a.path_score
                    .total_cmp(&b.path_score)
                    .then(a.hops.cmp(&b.hops))
                    .then(a.prob.total_cmp(&b.prob))
            });
            front.into_iter().take(k).collect()
        }
        "probability" => {
            scored.sort_by(|a, b| {
                a.prob
                    .total_cmp(&b.prob)
                    .then(a.path_score.total_cmp(&b.path_score))
                    .then(a.hops.cmp(&b.hops))
                    .then(a.sum_noise.total_cmp(&b.sum_noise))
            });
            scored.into_iter().take(k).collect()
        }
        _ => {
            scored.sort_by(|a, b| {
                a.path_score
                    .total_cmp(&b.path_score)
                    .then(a.hops.cmp(&b.hops))
                    .then(a.sum_noise.total_cmp(&b.sum_noise))
            });
            scored.into_iter().take(k).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InputEdge, InputNode, ScoredGraph};
    use std::collections::HashSet;

    fn node(id: &str) -> InputNode {
        InputNode {
            id: id.into(),
            name: Some(id.into()),
            node_type: Some("User".into()),
        }
    }
    fn edge(s: &str, t: &str, et: &str, noise: f64) -> InputEdge {
        InputEdge {
            source: s.into(),
            target: t.into(),
            edge_type: et.into(),
            noise,
            corpus_known: true,
        }
    }

    #[test]
    fn quiet_long_path_beats_loud_short_path() {
        // A: 2 x GenericAll(40) -> score 40.  B: 4 x AdminTo(25) -> score 25.
        // B has the higher weight-sum but is quieter; the threshold sweep must
        // still surface it. Same correctness backstop as the Python engine.
        let sg = ScoredGraph {
            nodes: vec![
                node("S"),
                node("A1"),
                node("D"),
                node("B1"),
                node("B2"),
                node("B3"),
            ],
            edges: vec![
                edge("S", "A1", "GenericAll", 40.0),
                edge("A1", "D", "GenericAll", 40.0),
                edge("S", "B1", "AdminTo", 25.0),
                edge("B1", "B2", "AdminTo", 25.0),
                edge("B2", "B3", "AdminTo", 25.0),
                edge("B3", "D", "AdminTo", 25.0),
            ],
        };
        let empty = HashSet::new();
        let g = Graph::build(&sg, &empty, &empty, &empty);
        let cfg = Config {
            max_weight: 0.6,
            mean_weight: 0.4,
            correlation: 0.5,
            candidates: 1,
            time_budget_s: 5.0,
        };
        let paths = solve(
            &g,
            g.resolve("S").unwrap(),
            g.resolve("D").unwrap(),
            5,
            "noise",
            &cfg,
        );
        assert_eq!(paths[0].hops, 4);
        assert!((paths[0].path_score - 25.0).abs() < 1e-6);
    }

    #[test]
    fn avoid_edge_type_reroutes() {
        let sg = ScoredGraph {
            nodes: vec![node("S"), node("D")],
            edges: vec![edge("S", "D", "HasSession", 20.0)],
        };
        let mut avoid = HashSet::new();
        avoid.insert("HasSession".to_string());
        let keep: HashSet<String> = ["S".into(), "D".into()].into_iter().collect();
        let g = Graph::build(&sg, &HashSet::new(), &avoid, &keep);
        let cfg = Config {
            max_weight: 0.6,
            mean_weight: 0.4,
            correlation: 0.5,
            candidates: 1,
            time_budget_s: 5.0,
        };
        // Only edge was avoided -> no path remains.
        assert!(solve(
            &g,
            g.resolve("S").unwrap(),
            g.resolve("D").unwrap(),
            5,
            "noise",
            &cfg
        )
        .is_empty());
    }

    fn measured_cfg() -> Config {
        Config {
            max_weight: 0.6,
            mean_weight: 0.4,
            correlation: 0.5,
            candidates: 5,
            time_budget_s: 5.0,
        }
    }

    #[test]
    fn measured_audit_profile_flips_to_adcs_esc1() {
        // Regression on the lab-measured AUDIT profile (profiles/vulnad-hyperv-audit.json)
        // and docs/VALIDATION.md: with measured HasSession=65 / AdminTo=34 / ADCSESC1=42,
        // the 1-hop ADCS ESC1 route (42.0) must beat the 4-hop session-hijack chain (49.3).
        // Proves DeadAir reproduces the Python engine's calibrated decision.
        let sg = ScoredGraph {
            nodes: vec![
                node("ALICE"),
                node("DA"),
                node("HELPDESK"),
                node("WS"),
                node("SVC"),
            ],
            edges: vec![
                edge("ALICE", "DA", "ADCSESC1", 42.0),
                edge("ALICE", "HELPDESK", "MemberOf", 2.0),
                edge("HELPDESK", "WS", "AdminTo", 34.0),
                edge("WS", "SVC", "HasSession", 65.0),
                edge("SVC", "DA", "MemberOf", 2.0),
            ],
        };
        let empty = HashSet::new();
        let g = Graph::build(&sg, &empty, &empty, &empty);
        let paths = solve(
            &g,
            g.resolve("ALICE").unwrap(),
            g.resolve("DA").unwrap(),
            5,
            "noise",
            &measured_cfg(),
        );
        assert_eq!(paths[0].hops, 1);
        assert!((paths[0].path_score - 42.0).abs() < 1e-6);
        assert_eq!(paths[1].hops, 4);
        assert!((paths[1].path_score - 49.3).abs() < 1e-6);
    }

    #[test]
    fn measured_edr_dcsync_is_the_loud_option() {
        // Regression on the lab-measured EDR profile: DCSync bumps to 85, so the quiet
        // 2-hop ADCS enrollment route (Enroll 36 -> ADCSESC1 42 = 40.8) must be preferred
        // over the 1-hop DCSync (85). Locks the measured EDR value into the engine.
        let sg = ScoredGraph {
            nodes: vec![node("ALICE"), node("DA"), node("CA")],
            edges: vec![
                edge("ALICE", "DA", "DCSync", 85.0),
                edge("ALICE", "CA", "Enroll", 36.0),
                edge("CA", "DA", "ADCSESC1", 42.0),
            ],
        };
        let empty = HashSet::new();
        let g = Graph::build(&sg, &empty, &empty, &empty);
        let paths = solve(
            &g,
            g.resolve("ALICE").unwrap(),
            g.resolve("DA").unwrap(),
            5,
            "noise",
            &measured_cfg(),
        );
        assert_eq!(paths[0].hops, 2);
        assert!((paths[0].path_score - 40.8).abs() < 1e-6);
        // the DCSync single hop is present but louder
        let dcsync = paths.iter().find(|p| p.hops == 1).unwrap();
        assert!((dcsync.path_score - 85.0).abs() < 1e-6);
    }
}
