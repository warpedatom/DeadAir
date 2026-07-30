//! DeadAir - fast noise-weighted Active Directory attack-path solver.
//!
//! The native engine core for NoiseHound. Reads a scored-graph JSON (nodes plus
//! edges carrying an effective noise score), finds the quietest paths from a
//! source to an objective, and emits the same ranked-paths schema NoiseHound's
//! own solver produces - just far faster on large graphs.

mod kshortest;
mod model;
mod solver;

use std::collections::HashSet;
use std::io::Read;

use clap::Parser;

use model::{OutEdge, OutPath, Output, ScoredGraph};
use solver::{solve, Config, Graph};

#[derive(Parser)]
#[command(
    name = "deadair",
    version,
    about = "Fast noise-weighted AD attack-path solver (NoiseHound engine core)"
)]
struct Args {
    /// Scored-graph JSON: a file path, or - for stdin.
    #[arg(short, long)]
    input: String,
    /// Source principal (object id or name).
    #[arg(short, long)]
    source: String,
    /// Objective node (object id or name, e.g. "Domain Admins").
    #[arg(short, long)]
    objective: String,
    /// Number of paths to return.
    #[arg(short = 'k', long, default_value_t = 5)]
    paths: usize,
    /// Ranking mode: noise | probability | pareto.
    #[arg(long, default_value = "noise")]
    mode: String,
    #[arg(long, default_value_t = 0.6)]
    max_weight: f64,
    #[arg(long, default_value_t = 0.4)]
    mean_weight: f64,
    #[arg(long, default_value_t = 0.5)]
    correlation: f64,
    #[arg(long, default_value_t = 200)]
    candidates: usize,
    /// Wall-clock budget (seconds) for the k-shortest pass.
    #[arg(long, default_value_t = 10.0)]
    time_budget: f64,
    /// Exclude a node from all paths (repeatable).
    #[arg(long)]
    avoid: Vec<String>,
    /// Exclude an edge type from all paths (repeatable).
    #[arg(long = "avoid-edge")]
    avoid_edge: Vec<String>,
    /// Print parse/solve timings to stderr.
    #[arg(long)]
    timing: bool,
}

fn read_input(path: &str) -> std::io::Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path)
    }
}

fn main() {
    let args = Args::parse();

    let raw = match read_input(&args.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("input error: {e}");
            std::process::exit(2);
        }
    };
    let t_parse = std::time::Instant::now();
    let sg: ScoredGraph = match serde_json::from_str(&raw) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("json error: {e}");
            std::process::exit(2);
        }
    };
    let parse_ms = t_parse.elapsed().as_secs_f64() * 1000.0;

    let empty: HashSet<String> = HashSet::new();
    let full = Graph::build(&sg, &empty, &empty, &empty);

    let src0 = match full.resolve(&args.source) {
        Some(i) => i,
        None => {
            eprintln!("source not found: {}", args.source);
            std::process::exit(3);
        }
    };
    let dst0 = match full.resolve(&args.objective) {
        Some(i) => i,
        None => {
            eprintln!("objective not found: {}", args.objective);
            std::process::exit(3);
        }
    };

    // Rebuild with constraints if any, keeping source/objective, then re-resolve.
    let (g, src, dst) = if args.avoid.is_empty() && args.avoid_edge.is_empty() {
        (full, src0, dst0)
    } else {
        let src_id = full.ids[src0].clone();
        let dst_id = full.ids[dst0].clone();
        let mut avoid_ids: HashSet<String> = HashSet::new();
        for a in &args.avoid {
            if let Some(i) = full.resolve(a) {
                avoid_ids.insert(full.ids[i].clone());
            }
        }
        let avoid_edges: HashSet<String> = args.avoid_edge.iter().cloned().collect();
        let keep: HashSet<String> = [src_id.clone(), dst_id.clone()].into_iter().collect();
        let g = Graph::build(&sg, &avoid_ids, &avoid_edges, &keep);
        let s = g.resolve(&src_id).expect("kept source");
        let d = g.resolve(&dst_id).expect("kept objective");
        (g, s, d)
    };

    let cfg = Config {
        max_weight: args.max_weight,
        mean_weight: args.mean_weight,
        correlation: args.correlation,
        candidates: args.candidates,
        time_budget_s: args.time_budget,
    };

    let t_solve = std::time::Instant::now();
    let scored = solve(&g, src, dst, args.paths, &args.mode, &cfg);
    if args.timing {
        eprintln!(
            "parse={:.1}ms solve={:.1}ms",
            parse_ms,
            t_solve.elapsed().as_secs_f64() * 1000.0
        );
    }

    let paths: Vec<OutPath> = scored
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let edges = s
                .nodes
                .windows(2)
                .map(|w| {
                    let e = &g.edges[&(w[0], w[1])];
                    OutEdge {
                        from: g.names[w[0]].clone(),
                        to: g.names[w[1]].clone(),
                        edge_type: e.edge_type.clone(),
                        noise: (e.noise * 10.0).round() / 10.0,
                        corpus_known: e.known,
                    }
                })
                .collect();
            OutPath {
                rank: i + 1,
                path_score: (s.path_score * 10.0).round() / 10.0,
                detection_probability: (s.prob * 1000.0).round() / 1000.0,
                hop_count: s.hops,
                edges,
            }
        })
        .collect();

    let out = Output {
        tool: "DeadAir".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        source: args.source,
        objective: args.objective,
        mode: args.mode,
        paths,
    };

    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
