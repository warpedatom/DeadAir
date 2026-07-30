//! Serde models for the scored-graph input and the ranked-paths output.
//!
//! DeadAir is corpus-agnostic: NoiseHound (Python) does ingestion, corpus
//! annotation, environment/Sigma adjustments, and hands DeadAir a *scored
//! graph* - nodes plus edges that already carry an effective noise score.
//! DeadAir does the heavy pathfinding and emits the same result schema
//! NoiseHound's own solver produces.

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct InputNode {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub node_type: Option<String>,
}

#[derive(Deserialize)]
pub struct InputEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    /// Effective noise score (0-100) already computed by NoiseHound.
    pub noise: f64,
    #[serde(default)]
    pub corpus_known: bool,
}

#[derive(Deserialize)]
pub struct ScoredGraph {
    pub nodes: Vec<InputNode>,
    pub edges: Vec<InputEdge>,
}

#[derive(Serialize)]
pub struct OutEdge {
    pub from: String,
    pub to: String,
    pub edge_type: String,
    pub noise: f64,
    pub corpus_known: bool,
}

#[derive(Serialize)]
pub struct OutPath {
    pub rank: usize,
    pub path_score: f64,
    pub detection_probability: f64,
    pub hop_count: usize,
    pub edges: Vec<OutEdge>,
}

#[derive(Serialize)]
pub struct Output {
    pub tool: String,
    pub version: String,
    pub source: String,
    pub objective: String,
    pub mode: String,
    pub paths: Vec<OutPath>,
}
