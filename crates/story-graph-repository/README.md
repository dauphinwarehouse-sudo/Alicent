# alicent-story-graph-repository

SQLite persistence adapter for `alicent-story-graph`.

## Guarantees

- A graph snapshot is read from one transaction and replaced in one transaction.
- Optimistic revision guards reject stale reads and writes.
- Domain constructors are used when rows are hydrated.
- Dangling entity references, parent cycles, kind mismatches, duplicate IDs and cross-kind UUID collisions are rejected.
- Canonical rows are ordered by stable IDs; children, timeline and backlink indexes are rebuilt by `StoryGraph` rather than persisted.
- A failed write rolls back both data and the revision increment.

## Standalone check

```sh
cargo test --manifest-path crates/story-graph-repository/Cargo.toml
cargo clippy --manifest-path crates/story-graph-repository/Cargo.toml --all-targets -- -D warnings
```

The crate deliberately contains an empty `[workspace]` table and is not listed in the repository root workspace yet. Integration must remove that table, add `crates/story-graph-repository` to the root workspace members/default-members as appropriate, and regenerate the root `Cargo.lock` in a dedicated integration change.
