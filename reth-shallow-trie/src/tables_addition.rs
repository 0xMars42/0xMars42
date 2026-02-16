// =============================================================================
// INSERT THE FOLLOWING TWO TABLE DEFINITIONS into the `tables!` macro in
// crates/storage/db-api/src/tables/mod.rs
//
// Insert AccountsTrieShallow RIGHT AFTER AccountsTrie.
// Insert StoragesTrieShallow RIGHT AFTER StoragesTrie.
// =============================================================================

// --- After AccountsTrie { ... } ---

    /// Shallow account trie nodes (path nibble count <= SHALLOW_TRIE_DEPTH).
    ///
    /// Dedicated table for hot top-level trie nodes that are accessed on nearly
    /// every state read. Separating these allows dedicated cache allocation and
    /// reduces compaction interference with deeper, less frequently accessed nodes.
    ///
    /// See: <https://github.com/paradigmxyz/reth/issues/21183>
    table AccountsTrieShallow {
        type Key = StoredNibbles;
        type Value = BranchNodeCompact;
    }

// --- After StoragesTrie { ... } ---

    /// Shallow storage trie nodes (path nibble count <= SHALLOW_TRIE_DEPTH).
    ///
    /// Dedicated table for hot top-level storage trie nodes. Same optimization
    /// rationale as `AccountsTrieShallow`.
    ///
    /// See: <https://github.com/paradigmxyz/reth/issues/21183>
    table StoragesTrieShallow {
        type Key = B256;
        type Value = StorageTrieEntry;
        type SubKey = StoredNibblesSubKey;
    }
