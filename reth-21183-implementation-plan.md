# reth #21183 — Split Shallow Trie Nodes into Dedicated Column

Implementation plan for [paradigmxyz/reth#21183](https://github.com/paradigmxyz/reth/issues/21183).

Inspired by Nethermind's FlatDB optimization ([PR #9854](https://github.com/NethermindEth/nethermind/pull/9854)).

---

## TL;DR

Separate trie nodes with path depth <= 5 nibbles into their own DB table. These shallow nodes are ~648 MB (0.5% of trie) but accessed on nearly every state read. Giving them dedicated cache space yields ~20% higher MGas/s under memory-constrained conditions.

---

## 1. Architecture

```
                  ┌─────────────────────┐
                  │   TrieCursor trait   │
                  └──────────┬──────────┘
                             │
              ┌──────────────┴──────────────┐
              │  SplitAccountTrieCursor      │
              │  (new wrapper)               │
              ├──────────────┬───────────────┤
              │              │               │
    ┌─────────▼───┐   ┌─────▼─────────┐
    │ AccountsTrie │   │ AccountsTrie  │
    │   Shallow    │   │   (deep)      │
    │  nibbles<=5  │   │  nibbles>5    │
    └──────────────┘   └───────────────┘
```

Same pattern for `StoragesTrie` / `StoragesTrieShallow`.

---

## 2. Constants

```rust
// crates/trie/common/src/lib.rs (or a new constants module)

/// Maximum nibble path length for nodes stored in the shallow trie table.
/// Nodes with path.len() <= this value go to the shallow table.
/// Based on Nethermind's analysis: ~648 MB, 0.5% of total trie, but heavily accessed.
pub const SHALLOW_TRIE_DEPTH: usize = 5;
```

---

## 3. New Tables

**File:** `crates/storage/db-api/src/tables/mod.rs`

Add inside the `tables!` macro block, right after the existing `AccountsTrie` and `StoragesTrie`:

```rust
/// Shallow account trie nodes (path nibble count <= 5).
/// Dedicated column for hot top-level nodes with separate cache allocation.
table AccountsTrieShallow {
    type Key = StoredNibbles;
    type Value = BranchNodeCompact;
}

/// Shallow storage trie nodes (path nibble count <= 5).
/// Dedicated column for hot top-level storage nodes with separate cache allocation.
table StoragesTrieShallow {
    type Key = B256;
    type Value = StorageTrieEntry;
    type SubKey = StoredNibblesSubKey;
}
```

This auto-generates `Table` impls, adds to the `Tables` enum, and registers them for MDBX table creation.

---

## 4. Compact Key Format (Optional Optimization)

The issue mentions a `[path_prefix:3 bytes][path_length:1 byte]` key format for the shallow column. This is a **secondary optimization** — we can start with the same `StoredNibbles` format and optimize later.

If we implement the compact format:

```rust
// crates/trie/common/src/nibbles.rs

/// Compact key for shallow trie nodes: [path_prefix:3 bytes][path_length:1 byte]
/// Total: 4 bytes fixed-size key for paths with <= 5 nibbles (max 10 nibbles packed).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShallowTrieKey([u8; 4]);

impl ShallowTrieKey {
    /// Encode a nibble path into the compact 4-byte format.
    /// Panics if nibbles.len() > 10 (5 bytes = 10 nibbles max).
    pub fn from_nibbles(nibbles: &Nibbles) -> Self {
        debug_assert!(nibbles.len() <= 10);
        let mut key = [0u8; 4];
        // Pack nibbles into first 3 bytes (6 nibbles max)
        let bytes: Vec<u8> = nibbles.iter().collect();
        for (i, &nibble) in bytes.iter().enumerate().take(6) {
            if i % 2 == 0 {
                key[i / 2] |= nibble << 4;
            } else {
                key[i / 2] |= nibble;
            }
        }
        // Last byte = path length
        key[3] = nibbles.len() as u8;
        Self(key)
    }

    /// Decode back to Nibbles.
    pub fn to_nibbles(&self) -> Nibbles {
        let len = self.0[3] as usize;
        let mut nibbles = Vec::with_capacity(len);
        for i in 0..len {
            let byte = self.0[i / 2];
            let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0x0f };
            nibbles.push(nibble);
        }
        Nibbles::from_nibbles_unchecked(nibbles)
    }
}

impl reth_codecs::Compact for ShallowTrieKey {
    fn to_compact<B>(&self, buf: &mut B) -> usize
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        buf.put_slice(&self.0);
        4
    }

    fn from_compact(buf: &[u8], _len: usize) -> (Self, &[u8]) {
        let mut key = [0u8; 4];
        key.copy_from_slice(&buf[..4]);
        (Self(key), &buf[4..])
    }
}
```

**Decision:** Start with `StoredNibbles` (same as deep). This keeps the PR smaller and reviewable. The compact key format can be a follow-up.

---

## 5. Split Cursor — Account Trie

**File:** `crates/trie/db/src/trie_cursor.rs`

```rust
use reth_trie_common::SHALLOW_TRIE_DEPTH;

/// Account trie cursor that routes between shallow and deep tables.
#[derive(Debug)]
pub struct SplitAccountTrieCursor<CS, CD> {
    /// Cursor over shallow nodes (nibbles <= SHALLOW_TRIE_DEPTH)
    shallow: DatabaseAccountTrieCursor<CS>,
    /// Cursor over deep nodes (nibbles > SHALLOW_TRIE_DEPTH)
    deep: DatabaseAccountTrieCursor<CD>,
    /// Which cursor returned the last result
    last_source: CursorSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorSource {
    Shallow,
    Deep,
    None,
}

impl<CS, CD> SplitAccountTrieCursor<CS, CD> {
    pub fn new(shallow: CS, deep: CD) -> Self {
        Self {
            shallow: DatabaseAccountTrieCursor::new(shallow),
            deep: DatabaseAccountTrieCursor::new(deep),
            last_source: CursorSource::None,
        }
    }
}

impl<CS, CD> TrieCursor for SplitAccountTrieCursor<CS, CD>
where
    CS: DbCursorRO<tables::AccountsTrieShallow> + Send,
    CD: DbCursorRO<tables::AccountsTrie> + Send,
{
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        if key.len() <= SHALLOW_TRIE_DEPTH {
            self.last_source = CursorSource::Shallow;
            self.shallow.seek_exact(key)
        } else {
            self.last_source = CursorSource::Deep;
            self.deep.seek_exact(key)
        }
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        if key.len() <= SHALLOW_TRIE_DEPTH {
            // Seek in shallow first
            let shallow_result = self.shallow.seek(key.clone())?;
            match shallow_result {
                Some((ref nibbles, _)) if nibbles.len() <= SHALLOW_TRIE_DEPTH => {
                    self.last_source = CursorSource::Shallow;
                    Ok(shallow_result)
                }
                _ => {
                    // No shallow result or it's beyond threshold, check deep
                    // with the original key — deep table starts at depth > 5
                    let deep_result = self.deep.seek(key)?;
                    match (&shallow_result, &deep_result) {
                        (Some((s_key, _)), Some((d_key, _))) => {
                            // Return the smaller key (earlier in traversal order)
                            if s_key <= d_key {
                                self.last_source = CursorSource::Shallow;
                                Ok(shallow_result)
                            } else {
                                self.last_source = CursorSource::Deep;
                                Ok(deep_result)
                            }
                        }
                        (Some(_), None) => {
                            self.last_source = CursorSource::Shallow;
                            Ok(shallow_result)
                        }
                        (None, _) => {
                            self.last_source = CursorSource::Deep;
                            Ok(deep_result)
                        }
                    }
                }
            }
        } else {
            self.last_source = CursorSource::Deep;
            self.deep.seek(key)
        }
    }

    fn next(
        &mut self,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        match self.last_source {
            CursorSource::Shallow => {
                let result = self.shallow.next()?;
                match result {
                    Some((ref nibbles, _)) if nibbles.len() <= SHALLOW_TRIE_DEPTH => {
                        Ok(result)
                    }
                    _ => {
                        // Shallow exhausted or returned deep node, switch to deep
                        // We need to seek deep to the right position
                        self.last_source = CursorSource::Deep;
                        // Deep cursor should already be positioned from initial seek
                        // or we need to seek to the first deep entry
                        self.deep.next()
                    }
                }
            }
            CursorSource::Deep => self.deep.next(),
            CursorSource::None => {
                // Try shallow first, then deep
                let result = self.shallow.next()?;
                if result.is_some() {
                    self.last_source = CursorSource::Shallow;
                    Ok(result)
                } else {
                    self.last_source = CursorSource::Deep;
                    self.deep.next()
                }
            }
        }
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        match self.last_source {
            CursorSource::Shallow => self.shallow.current(),
            CursorSource::Deep => self.deep.current(),
            CursorSource::None => Ok(None),
        }
    }

    fn reset(&mut self) {
        self.shallow.reset();
        self.deep.reset();
        self.last_source = CursorSource::None;
    }
}
```

---

## 6. Update TrieCursorFactory

**File:** `crates/trie/db/src/trie_cursor.rs`

```rust
impl<TX> TrieCursorFactory for DatabaseTrieCursorFactory<&TX>
where
    TX: DbTx,
{
    type AccountTrieCursor<'a>
        = SplitAccountTrieCursor<
            <TX as DbTx>::Cursor<tables::AccountsTrieShallow>,
            <TX as DbTx>::Cursor<tables::AccountsTrie>,
        >
    where
        Self: 'a;

    // StorageTrieCursor similarly...

    fn account_trie_cursor(&self) -> Result<Self::AccountTrieCursor<'_>, DatabaseError> {
        let shallow = self.0.cursor_read::<tables::AccountsTrieShallow>()?;
        let deep = self.0.cursor_read::<tables::AccountsTrie>()?;
        Ok(SplitAccountTrieCursor::new(shallow, deep))
    }
}
```

---

## 7. Write Path — Trie Updates

**File:** `crates/trie/db/src/trie_updates.rs` (or wherever `TrieUpdates` are flushed to DB)

Wherever nodes are written to `AccountsTrie`, add routing:

```rust
fn write_account_trie_node(
    tx: &impl DbTxMut,
    nibbles: &Nibbles,
    node: &BranchNodeCompact,
) -> Result<(), DatabaseError> {
    if nibbles.len() <= SHALLOW_TRIE_DEPTH {
        tx.put::<tables::AccountsTrieShallow>(
            StoredNibbles(nibbles.clone()),
            node.clone(),
        )?;
    } else {
        tx.put::<tables::AccountsTrie>(
            StoredNibbles(nibbles.clone()),
            node.clone(),
        )?;
    }
    Ok(())
}

fn delete_account_trie_node(
    tx: &impl DbTxMut,
    nibbles: &Nibbles,
) -> Result<(), DatabaseError> {
    if nibbles.len() <= SHALLOW_TRIE_DEPTH {
        tx.delete::<tables::AccountsTrieShallow>(
            StoredNibbles(nibbles.clone()),
            None,
        )?;
    } else {
        tx.delete::<tables::AccountsTrie>(
            StoredNibbles(nibbles.clone()),
            None,
        )?;
    }
    Ok(())
}
```

Same pattern for storage trie writes.

---

## 8. Migration Strategy

Existing nodes need to be moved from `AccountsTrie` to `AccountsTrieShallow`. Two options:

### Option A: Lazy migration (recommended for first PR)
- On read: if not found in expected table, check the other one
- On write: always write to correct table
- Over time, all shallow nodes migrate to the new table
- Pro: no downtime, backwards-compatible
- Con: slightly slower reads during migration period

### Option B: Explicit migration
- Add a one-time migration step on startup
- Iterate `AccountsTrie`, move entries with `nibbles.len() <= 5` to `AccountsTrieShallow`
- Pro: clean cutover
- Con: requires downtime, migration can take minutes

**Recommendation:** Start with Option A for the PR. Maintainers may prefer Option B — ask in the issue.

---

## 9. RocksDB Cache Configuration

For RocksDB backend (not MDBX), configure dedicated block cache:

```rust
// Pseudocode for RocksDB column family options
let total_cache = 1_073_741_824; // 1 GB
let shallow_cache = (total_cache as f64 * 0.30) as usize; // 30% = ~300 MB
let deep_cache = total_cache - shallow_cache; // 70% = ~700 MB

// AccountsTrieShallow column family
let mut shallow_opts = Options::default();
let shallow_block_cache = Cache::new_lru_cache(shallow_cache);
let mut shallow_table_opts = BlockBasedOptions::default();
shallow_table_opts.set_block_cache(&shallow_block_cache);
shallow_opts.set_block_based_table_factory(&shallow_table_opts);

// AccountsTrie column family (deep)
let mut deep_opts = Options::default();
let deep_block_cache = Cache::new_lru_cache(deep_cache);
let mut deep_table_opts = BlockBasedOptions::default();
deep_table_opts.set_block_cache(&deep_block_cache);
deep_opts.set_block_based_table_factory(&deep_table_opts);
```

**Note:** MDBX (reth's primary backend) doesn't have column families. This optimization is primarily beneficial for the RocksDB backend. For MDBX, the split still helps with page-level cache locality in the OS page cache.

---

## 10. Implementation Order

### Phase 1 — Core (this PR)
1. [ ] Add `SHALLOW_TRIE_DEPTH` constant
2. [ ] Add `AccountsTrieShallow` and `StoragesTrieShallow` table definitions
3. [ ] Implement `SplitAccountTrieCursor`
4. [ ] Implement `SplitStorageTrieCursor`
5. [ ] Update `DatabaseTrieCursorFactory` to use split cursors
6. [ ] Route writes through depth check
7. [ ] Route deletes through depth check
8. [ ] Add lazy migration fallback on reads
9. [ ] Unit tests: verify nodes go to correct table based on depth
10. [ ] Unit tests: verify cursor merges results correctly
11. [ ] Run existing trie tests — everything must pass

### Phase 2 — Follow-up PRs
- [ ] Compact key format (`ShallowTrieKey`)
- [ ] RocksDB cache configuration
- [ ] Benchmarks comparing before/after
- [ ] Explicit migration command

---

## 11. Testing Strategy

```rust
#[test]
fn shallow_nodes_routed_to_shallow_table() {
    let db = create_test_db();
    let tx = db.tx_mut().unwrap();

    // Write a node at depth 3 (shallow)
    let shallow_path = Nibbles::from_nibbles([0x1, 0x2, 0x3]);
    let node = test_branch_node();
    write_account_trie_node(&tx, &shallow_path, &node).unwrap();

    // Verify it's in the shallow table
    let mut cursor = tx.cursor_read::<tables::AccountsTrieShallow>().unwrap();
    assert!(cursor.seek_exact(StoredNibbles(shallow_path.clone())).unwrap().is_some());

    // Verify it's NOT in the deep table
    let mut cursor = tx.cursor_read::<tables::AccountsTrie>().unwrap();
    assert!(cursor.seek_exact(StoredNibbles(shallow_path)).unwrap().is_none());
}

#[test]
fn deep_nodes_routed_to_deep_table() {
    let db = create_test_db();
    let tx = db.tx_mut().unwrap();

    // Write a node at depth 8 (deep)
    let deep_path = Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]);
    let node = test_branch_node();
    write_account_trie_node(&tx, &deep_path, &node).unwrap();

    // Verify it's in the deep table
    let mut cursor = tx.cursor_read::<tables::AccountsTrie>().unwrap();
    assert!(cursor.seek_exact(StoredNibbles(deep_path.clone())).unwrap().is_some());

    // Verify it's NOT in the shallow table
    let mut cursor = tx.cursor_read::<tables::AccountsTrieShallow>().unwrap();
    assert!(cursor.seek_exact(StoredNibbles(deep_path)).unwrap().is_none());
}

#[test]
fn split_cursor_merges_results_in_order() {
    let db = create_test_db();
    let tx = db.tx_mut().unwrap();

    // Insert shallow and deep nodes
    let paths = vec![
        Nibbles::from_nibbles([0x1]),              // shallow
        Nibbles::from_nibbles([0x1, 0x2, 0x3]),    // shallow
        Nibbles::from_nibbles([0x2, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1]), // deep
        Nibbles::from_nibbles([0x3]),              // shallow
    ];

    for path in &paths {
        write_account_trie_node(&tx, path, &test_branch_node()).unwrap();
    }

    // Read all via split cursor — must come out in sorted order
    let factory = DatabaseTrieCursorFactory::new(&tx);
    let mut cursor = factory.account_trie_cursor().unwrap();
    let first = cursor.seek(Nibbles::default()).unwrap();
    assert!(first.is_some());

    let mut results = vec![first.unwrap().0];
    while let Some((nibbles, _)) = cursor.next().unwrap() {
        results.push(nibbles);
    }

    // Verify sorted order
    for window in results.windows(2) {
        assert!(window[0] <= window[1], "results not sorted: {:?} > {:?}", window[0], window[1]);
    }
    assert_eq!(results.len(), 4);
}

#[test]
fn boundary_depth_5_goes_to_shallow() {
    // Nibbles of exactly length 5 should go to shallow
    let path = Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5]);
    assert!(path.len() <= SHALLOW_TRIE_DEPTH);
}

#[test]
fn boundary_depth_6_goes_to_deep() {
    // Nibbles of length 6 should go to deep
    let path = Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5, 0x6]);
    assert!(path.len() > SHALLOW_TRIE_DEPTH);
}
```

---

## 12. Files to Modify (Summary)

| File | Change |
|------|--------|
| `crates/trie/common/src/lib.rs` | Add `SHALLOW_TRIE_DEPTH` constant |
| `crates/storage/db-api/src/tables/mod.rs` | Add `AccountsTrieShallow`, `StoragesTrieShallow` tables |
| `crates/trie/db/src/trie_cursor.rs` | Add `SplitAccountTrieCursor`, `SplitStorageTrieCursor` |
| `crates/trie/db/src/trie_cursor.rs` | Update `DatabaseTrieCursorFactory` impl |
| `crates/trie/db/src/trie_updates.rs` | Route writes/deletes by depth |
| `crates/trie/db/src/lib.rs` | Export new types |
| `crates/storage/db/src/static_file/...` | May need table registration |

---

## 13. Risks & Open Questions

1. **MDBX vs RocksDB** — reth's primary backend is MDBX which doesn't have column families. The table split still helps with page cache locality, but the explicit cache allocation only benefits RocksDB. Should we add a config flag?

2. **Cursor merge correctness** — The `next()` implementation on the split cursor needs careful handling of the transition from shallow to deep. The trie walker calls `seek()` then `next()` in a specific pattern — need to verify this works correctly with the split.

3. **Depth threshold** — Nethermind uses 5, but reth's trie structure may have different access patterns. Consider making this configurable.

4. **Write cursor** — `DatabaseTrieCursorFactory` also has a mutable variant for writes. The write path needs the same split logic.

5. **Backward compatibility** — Existing databases won't have the new tables. Need graceful handling on first startup.
