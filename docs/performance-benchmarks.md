# Performance benchmarks

`P0.1` uses deterministic synthetic Russian text and the production repository API. Generated
projects are temporary by default and contain no user data.

## Storage benchmark

Run an optimized build on a local, non-synchronised disk:

```sh
cargo run --locked --release -p alicent-project-repository \
  --example performance -- --profile smoke --output target/benchmarks/smoke.json
cargo run --locked --release -p alicent-project-repository \
  --example performance -- --profile medium --output target/benchmarks/medium.json
cargo run --locked --release -p alicent-project-repository \
  --example performance -- --profile acceptance --output target/benchmarks/acceptance.json
```

| Profile | Total words | Nodes | Large document | Purpose |
|---|---:|---:|---:|---|
| `smoke` | 10,000 | 200 | 20,000 words | Fast regression check |
| `medium` | 500,000 | 2,000 | 200,000 words | Developer workstation check |
| `acceptance` | 5,000,000 | 20,000 | 200,000 words | Specification dataset |

The JSON report records dataset generation time, database size, current RSS on Linux, and p50/p95/max
for repository open, large-document read, common-term search and rare-term search. The first operation
is a warm-up and is not included.

These numbers cover SQLite/repository work only. Application startup, Windows installed-app memory and
visual input latency must be recorded separately on the declared acceptance PC. Do not label the
repository `open_ms` value as time-to-interactive.

## Recording an acceptance run

Record the date, OS/build, CPU, RAM, storage type, power mode, Rust/Node versions and exact commit.
Keep generated JSON reports out of source control unless they include that environment metadata.

Targets from the specification:

- application startup: at most 3 seconds;
- ordinary project to interactive: at most 2 seconds;
- visual input latency: less than 50 ms;
- first search results on the indexed 5-million-word project: less than 300 ms;
- opening one document must not make memory scale linearly with the whole project.