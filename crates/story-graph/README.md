# Alicent story graph prototype

Pure, in-memory domain prototype for P4.1. It models characters, places, typed
relations, timeline events, hierarchical lazy traversal, backlinks and
scope/excluded-context projection.

It intentionally has no SQLite adapter, migration, commands, desktop bindings,
provider code, import/export path or UI. A future persistence adapter must parse
storage DTOs and call the validating constructors; domain values are not directly
deserializable.

Run the isolated checks with:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
