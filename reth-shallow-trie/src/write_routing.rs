// =============================================================================
// Write routing for trie updates — route by depth
//
// These functions replace direct writes to AccountsTrie / StoragesTrie
// in the provider's `write_trie_updates_sorted` and
// `write_storage_trie_updates_sorted` methods.
//
// Target file: wherever `TrieUpdatesSorted` is flushed to DB, typically
// `crates/storage/provider/src/providers/database/provider.rs`
// =============================================================================

use reth_db_api::{
    cursor::{DbCursorRO, DbCursorRW, DbDupCursorRO, DbDupCursorRW},
    tables,
    transaction::DbTxMut,
    DatabaseError,
};
use reth_trie::{
    updates::{StorageTrieUpdatesSorted, TrieUpdatesSorted},
    BranchNodeCompact, Nibbles, StorageTrieEntry, StoredNibbles, StoredNibblesSubKey,
};
use reth_trie_common::constants::SHALLOW_TRIE_DEPTH;
use alloy_primitives::B256;

// =============================================================================
// Account trie write routing
// =============================================================================

/// Write sorted account trie updates, routing each node to the correct table
/// based on its nibble path depth.
///
/// This replaces the original loop in `write_trie_updates_sorted` that writes
/// directly to `AccountsTrie`.
pub fn write_account_trie_updates_split<TX: DbTxMut>(
    tx: &TX,
    updates: &TrieUpdatesSorted,
) -> Result<usize, DatabaseError> {
    let mut shallow_cursor = tx.cursor_write::<tables::AccountsTrieShallow>()?;
    let mut deep_cursor = tx.cursor_write::<tables::AccountsTrie>()?;
    let mut num_entries = 0;

    for (nibbles, maybe_node) in &updates.account_nodes {
        if nibbles.is_empty() {
            continue;
        }

        num_entries += 1;
        let stored = StoredNibbles(nibbles.clone());

        if nibbles.len() <= SHALLOW_TRIE_DEPTH {
            // --- Shallow table ---
            // Delete old entry if it exists.
            if shallow_cursor.seek_exact(stored.clone())?.is_some() {
                shallow_cursor.delete_current()?;
            }
            // Insert updated node (None = deletion, no insert needed).
            if let Some(node) = maybe_node {
                shallow_cursor.upsert(stored, node)?;
            }
        } else {
            // --- Deep table ---
            if deep_cursor.seek_exact(stored.clone())?.is_some() {
                deep_cursor.delete_current()?;
            }
            if let Some(node) = maybe_node {
                deep_cursor.upsert(stored, node)?;
            }
        }
    }

    Ok(num_entries)
}

// =============================================================================
// Storage trie write routing
// =============================================================================

/// Write sorted storage trie updates for a single account, routing each node
/// to the correct table based on its nibble path depth.
///
/// This replaces `DatabaseStorageTrieCursor::write_storage_trie_updates_sorted`
/// for the split-table layout.
pub fn write_storage_trie_updates_split<TX: DbTxMut>(
    tx: &TX,
    hashed_address: B256,
    updates: &StorageTrieUpdatesSorted,
) -> Result<usize, DatabaseError> {
    let mut shallow_cursor = tx.cursor_dup_write::<tables::StoragesTrieShallow>()?;
    let mut deep_cursor = tx.cursor_dup_write::<tables::StoragesTrie>()?;

    // If the entire storage trie for this account is deleted, clear both tables.
    if updates.is_deleted() {
        if shallow_cursor.seek_exact(hashed_address)?.is_some() {
            shallow_cursor.delete_current_duplicates()?;
        }
        if deep_cursor.seek_exact(hashed_address)?.is_some() {
            deep_cursor.delete_current_duplicates()?;
        }
    }

    let mut num_entries = 0;

    for (nibbles, maybe_node) in updates.storage_nodes.iter().filter(|(n, _)| !n.is_empty()) {
        num_entries += 1;
        let stored_nibbles = StoredNibblesSubKey(*nibbles);

        if nibbles.len() <= SHALLOW_TRIE_DEPTH {
            // --- Shallow table ---
            if shallow_cursor
                .seek_by_key_subkey(hashed_address, stored_nibbles.clone())?
                .filter(|e| e.nibbles == stored_nibbles)
                .is_some()
            {
                shallow_cursor.delete_current()?;
            }
            if let Some(node) = maybe_node {
                shallow_cursor.upsert(
                    hashed_address,
                    &StorageTrieEntry { nibbles: stored_nibbles, node: node.clone() },
                )?;
            }
        } else {
            // --- Deep table ---
            if deep_cursor
                .seek_by_key_subkey(hashed_address, stored_nibbles.clone())?
                .filter(|e| e.nibbles == stored_nibbles)
                .is_some()
            {
                deep_cursor.delete_current()?;
            }
            if let Some(node) = maybe_node {
                deep_cursor.upsert(
                    hashed_address,
                    &StorageTrieEntry { nibbles: stored_nibbles, node: node.clone() },
                )?;
            }
        }
    }

    Ok(num_entries)
}

// =============================================================================
// Clear helpers (for merkle stage full rebuild)
// =============================================================================

/// Clear both shallow and deep account trie tables.
/// Replaces `tx.clear::<tables::AccountsTrie>()?` in the merkle stage.
pub fn clear_account_trie_tables<TX: DbTxMut>(tx: &TX) -> Result<(), DatabaseError> {
    tx.clear::<tables::AccountsTrieShallow>()?;
    tx.clear::<tables::AccountsTrie>()?;
    Ok(())
}

/// Clear both shallow and deep storage trie tables.
/// Replaces `tx.clear::<tables::StoragesTrie>()?` in the merkle stage.
pub fn clear_storage_trie_tables<TX: DbTxMut>(tx: &TX) -> Result<(), DatabaseError> {
    tx.clear::<tables::StoragesTrieShallow>()?;
    tx.clear::<tables::StoragesTrie>()?;
    Ok(())
}
