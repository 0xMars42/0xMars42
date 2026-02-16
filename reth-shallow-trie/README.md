# reth #21183 — Split Shallow Trie Nodes: Implementation

Ready-to-apply source files for [paradigmxyz/reth#21183](https://github.com/paradigmxyz/reth/issues/21183).

## Architecture

```
              ┌──────────────────────┐
              │    TrieCursor trait   │
              └──────────┬───────────┘
                         │
          ┌──────────────┴──────────────┐
          │  SplitAccountTrieCursor     │
          │  (two-way sorted merge)     │
          ├──────────────┬──────────────┤
          │              │              │
 ┌────────▼────────┐  ┌─▼────────────┐
 │ AccountsTrie    │  │ AccountsTrie │
 │   Shallow       │  │   (deep)     │
 │ nibbles <= 5    │  │ nibbles > 5  │
 └─────────────────┘  └──────────────┘
```

Same pattern for `StoragesTrie` / `StoragesTrieShallow`.

## Files

| File | Target in reth | Description |
|------|---------------|-------------|
| `src/constants.rs` | `crates/trie/common/src/constants.rs` | `SHALLOW_TRIE_DEPTH = 5` constant |
| `src/tables_addition.rs` | `crates/storage/db-api/src/tables/mod.rs` | `AccountsTrieShallow` + `StoragesTrieShallow` table defs (insert into `tables!` macro) |
| `src/trie_cursor.rs` | `crates/trie/db/src/trie_cursor.rs` | Complete replacement — `SplitAccountTrieCursor`, `SplitStorageTrieCursor`, updated `DatabaseTrieCursorFactory` |
| `src/write_routing.rs` | New module or integrate into provider write path | `write_account_trie_updates_split`, `write_storage_trie_updates_split`, `clear_*_trie_tables` |

## Key design decisions

1. **Two-way sorted merge cursor** — Each split cursor maintains `pending_shallow` and `pending_deep` slots. On `seek()`, both underlying cursors are positioned and results buffered. On `next()`, only the previously-consumed cursor advances. `consume_smaller()` picks the winner. This guarantees sorted iteration without losing entries.

2. **Depth threshold** — `SHALLOW_TRIE_DEPTH = 5` nibbles, matching Nethermind's analysis. Nodes at this depth are ~648 MB (0.5% of trie) but accessed on nearly every state read.

3. **`seek_exact` optimization** — Since each key has a known depth, `seek_exact` goes directly to the correct table without buffering.

4. **Write routing** — `write_account_trie_updates_split` and `write_storage_trie_updates_split` replace the original single-table write loops, routing by `nibbles.len() <= SHALLOW_TRIE_DEPTH`.

5. **Storage cursor address switch** — `set_hashed_address()` clears both pending slots to avoid stale cross-account data.

## How to apply

```bash
RETH=/path/to/reth

# 1. Constants
cp src/constants.rs $RETH/crates/trie/common/src/constants.rs

# 2. Tables — manually insert the two table blocks from src/tables_addition.rs
#    into the `tables!` macro in $RETH/crates/storage/db-api/src/tables/mod.rs
#    (after AccountsTrie and StoragesTrie respectively)

# 3. Trie cursor — full replacement
cp src/trie_cursor.rs $RETH/crates/trie/db/src/trie_cursor.rs

# 4. Write routing — add as new module or integrate into provider
#    Replace calls to the old single-table write loop with
#    write_account_trie_updates_split() / write_storage_trie_updates_split()

# 5. Re-export SHALLOW_TRIE_DEPTH from crates/trie/common/src/lib.rs:
#    pub use constants::SHALLOW_TRIE_DEPTH;
```

## Testing

The `trie_cursor.rs` file includes tests for:
- `split_cursor_seek_exact_routes_by_depth` — verifies seek_exact goes to correct table
- `split_cursor_seek_then_next_produces_sorted_merge` — 5 interleaved entries, checks sorted order
- `split_cursor_empty_tables` — both empty, no panic
- `split_cursor_only_shallow_entries` / `split_cursor_only_deep_entries` — single-table scenarios
- `split_cursor_boundary_depth_5_is_shallow` / `split_cursor_boundary_depth_6_is_deep` — boundary
- Original tests preserved (`test_account_trie_order`, `test_storage_cursor_abstraction`)
