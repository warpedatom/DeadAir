# DeadAir

**Fast noise-weighted Active Directory attack-path solver - the native engine core for [NoiseHound](../NoiseHound).**
_DreadHost Research_

DeadAir is to NoiseHound what OffsetScan is to OffsetInspect: the compiled,
distributable engine. NoiseHound (Python) does ingestion, corpus annotation, and
environment/Sigma scoring, then hands DeadAir a *scored graph* - nodes plus edges
carrying an effective noise score. DeadAir finds the quietest paths and emits the
same ranked-paths JSON NoiseHound's own solver produces.

> "Dead air" - the route through no signal. For authorized security testing only.

## Build

```bash
cargo build --release
```

## Use

```bash
deadair --input scored_graph.json --source jdoe --objective "Domain Admins" -k 5
cat scored_graph.json | deadair -i - -s jdoe -o "Domain Admins" --mode pareto
```

Scored-graph input:

```json
{
  "nodes": [{"id": "S-1-...", "name": "jdoe@CORP", "type": "User"}],
  "edges": [{"source": "S-1-A", "target": "S-1-B", "edge_type": "GenericAll",
             "noise": 40.0, "corpus_known": true}]
}
```

Options: `--mode noise|probability|pareto`, `--max-weight` / `--mean-weight` /
`--correlation`, `--candidates`, `--avoid NODE`, `--avoid-edge TYPE`, `--timing`.

## Status (v0.1.0)

Produces **identical path rankings to NoiseHound's Python engine** (validated),
ships as a single static binary, and is **10-100x faster** on large graphs.

The k-shortest pass is a custom A*-accelerated, time-bounded Yen's
(`kshortest.rs`): a single reverse-Dijkstra precomputes distance-to-target, then
every spur search is an A* guided straight at the target instead of a blind
Dijkstra. Measured (50 candidates, pure solve time):

| Graph | NoiseHound (Python) | DeadAir | Speedup |
|-------|--------------------:|--------:|--------:|
| 40k nodes  | 5.3s  | 51ms  | 103x |
| 100k nodes | 10.0s | 452ms | 22x  |
| 250k nodes | 29.5s | 2.2s  | 14x  |

`--time-budget` bounds the k-shortest pass so huge graphs degrade gracefully;
the threshold-sweep backstop still guarantees a correct answer.

## Test

```bash
cargo test
```
