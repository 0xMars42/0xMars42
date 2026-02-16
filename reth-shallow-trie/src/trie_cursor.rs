use alloy_primitives::B256;
use reth_db_api::{
    cursor::{DbCursorRO, DbCursorRW, DbDupCursorRO, DbDupCursorRW},
    tables,
    transaction::DbTx,
    DatabaseError,
};
use reth_trie::{
    trie_cursor::{TrieCursor, TrieCursorFactory, TrieStorageCursor},
    updates::StorageTrieUpdatesSorted,
    BranchNodeCompact, Nibbles, StorageTrieEntry, StoredNibbles, StoredNibblesSubKey,
};
use reth_trie_common::constants::SHALLOW_TRIE_DEPTH;

// =============================================================================
// DatabaseTrieCursorFactory — now returns split cursors
// =============================================================================

/// Wrapper struct for database transaction implementing trie cursor factory trait.
#[derive(Debug, Clone)]
pub struct DatabaseTrieCursorFactory<T>(T);

impl<T> DatabaseTrieCursorFactory<T> {
    /// Create new [`DatabaseTrieCursorFactory`].
    pub const fn new(tx: T) -> Self {
        Self(tx)
    }
}

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

    type StorageTrieCursor<'a>
        = SplitStorageTrieCursor<
            <TX as DbTx>::DupCursor<tables::StoragesTrieShallow>,
            <TX as DbTx>::DupCursor<tables::StoragesTrie>,
        >
    where
        Self: 'a;

    fn account_trie_cursor(&self) -> Result<Self::AccountTrieCursor<'_>, DatabaseError> {
        let shallow = self.0.cursor_read::<tables::AccountsTrieShallow>()?;
        let deep = self.0.cursor_read::<tables::AccountsTrie>()?;
        Ok(SplitAccountTrieCursor::new(shallow, deep))
    }

    fn storage_trie_cursor(
        &self,
        hashed_address: B256,
    ) -> Result<Self::StorageTrieCursor<'_>, DatabaseError> {
        let shallow = self.0.cursor_dup_read::<tables::StoragesTrieShallow>()?;
        let deep = self.0.cursor_dup_read::<tables::StoragesTrie>()?;
        Ok(SplitStorageTrieCursor::new(shallow, deep, hashed_address))
    }
}

// =============================================================================
// CursorSource — tracks which cursor was last consumed
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorSource {
    Shallow,
    Deep,
}

// =============================================================================
// DatabaseAccountTrieCursor — unchanged single-table cursor
// =============================================================================

/// A cursor over the account trie (single table).
#[derive(Debug)]
pub struct DatabaseAccountTrieCursor<C>(pub(crate) C);

impl<C> DatabaseAccountTrieCursor<C> {
    /// Create a new account trie cursor.
    pub const fn new(cursor: C) -> Self {
        Self(cursor)
    }
}

impl<C> TrieCursor for DatabaseAccountTrieCursor<C>
where
    C: DbCursorRO<tables::AccountsTrie> + Send,
{
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.0.seek_exact(StoredNibbles(key))?.map(|value| (value.0 .0, value.1)))
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.0.seek(StoredNibbles(key))?.map(|value| (value.0 .0, value.1)))
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.0.next()?.map(|value| (value.0 .0, value.1)))
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        Ok(self.0.current()?.map(|(k, _)| k.0))
    }

    fn reset(&mut self) {
        // No-op for database cursors
    }
}

// =============================================================================
// ShallowAccountTrieCursor — typed for AccountsTrieShallow table
// =============================================================================

/// Cursor over the shallow account trie table.
#[derive(Debug)]
struct ShallowAccountTrieCursor<C>(C);

impl<C> ShallowAccountTrieCursor<C>
where
    C: DbCursorRO<tables::AccountsTrieShallow> + Send,
{
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.0.seek_exact(StoredNibbles(key))?.map(|v| (v.0 .0, v.1)))
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.0.seek(StoredNibbles(key))?.map(|v| (v.0 .0, v.1)))
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.0.next()?.map(|v| (v.0 .0, v.1)))
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        Ok(self.0.current()?.map(|(k, _)| k.0))
    }
}

// =============================================================================
// SplitAccountTrieCursor — correct sorted-merge over two tables
// =============================================================================

/// Account trie cursor that transparently merges results from the shallow
/// table (`AccountsTrieShallow`, nibbles <= [`SHALLOW_TRIE_DEPTH`]) and the
/// deep table (`AccountsTrie`, nibbles > [`SHALLOW_TRIE_DEPTH`]).
///
/// Uses the standard two-way merge pattern: each cursor has a "pending" slot.
/// On every `seek`/`next` we refill whichever slot was consumed, then return
/// the entry with the smaller key, leaving the other buffered for later.
#[derive(Debug)]
pub struct SplitAccountTrieCursor<CS, CD> {
    shallow: ShallowAccountTrieCursor<CS>,
    deep: DatabaseAccountTrieCursor<CD>,
    /// Buffered entry from the shallow cursor (not yet returned to caller).
    pending_shallow: Option<(Nibbles, BranchNodeCompact)>,
    /// Buffered entry from the deep cursor (not yet returned to caller).
    pending_deep: Option<(Nibbles, BranchNodeCompact)>,
    /// Which cursor was consumed on the last call (used by `current()`).
    last_consumed: Option<CursorSource>,
}

impl<CS, CD> SplitAccountTrieCursor<CS, CD> {
    /// Create a new split account trie cursor.
    pub fn new(shallow: CS, deep: CD) -> Self {
        Self {
            shallow: ShallowAccountTrieCursor(shallow),
            deep: DatabaseAccountTrieCursor::new(deep),
            pending_shallow: None,
            pending_deep: None,
            last_consumed: None,
        }
    }
}

impl<CS, CD> SplitAccountTrieCursor<CS, CD>
where
    CS: DbCursorRO<tables::AccountsTrieShallow> + Send,
    CD: DbCursorRO<tables::AccountsTrie> + Send,
{
    /// Compare the two pending slots and consume (return) the smaller entry.
    /// The other entry stays buffered.
    fn consume_smaller(&mut self) -> Option<(Nibbles, BranchNodeCompact)> {
        match (&self.pending_shallow, &self.pending_deep) {
            (Some((s, _)), Some((d, _))) => {
                if s <= d {
                    self.last_consumed = Some(CursorSource::Shallow);
                    self.pending_shallow.take()
                } else {
                    self.last_consumed = Some(CursorSource::Deep);
                    self.pending_deep.take()
                }
            }
            (Some(_), None) => {
                self.last_consumed = Some(CursorSource::Shallow);
                self.pending_shallow.take()
            }
            (None, Some(_)) => {
                self.last_consumed = Some(CursorSource::Deep);
                self.pending_deep.take()
            }
            (None, None) => {
                self.last_consumed = None;
                None
            }
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
        // seek_exact targets a specific key — it lives in exactly one table
        self.pending_shallow = None;
        self.pending_deep = None;

        if key.len() <= SHALLOW_TRIE_DEPTH {
            self.last_consumed = Some(CursorSource::Shallow);
            self.shallow.seek_exact(key)
        } else {
            self.last_consumed = Some(CursorSource::Deep);
            self.deep.seek_exact(key)
        }
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        // Seek in both tables, buffer both results, return the smaller.
        self.pending_shallow = self.shallow.seek(key.clone())?;
        self.pending_deep = self.deep.seek(key)?;
        Ok(self.consume_smaller())
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        // Refill whichever slot was consumed on the previous call.
        match self.last_consumed {
            Some(CursorSource::Shallow) => {
                self.pending_shallow = self.shallow.next()?;
            }
            Some(CursorSource::Deep) => {
                self.pending_deep = self.deep.next()?;
            }
            None => {
                // First call to next() without a preceding seek — fill both.
                if self.pending_shallow.is_none() {
                    self.pending_shallow = self.shallow.next()?;
                }
                if self.pending_deep.is_none() {
                    self.pending_deep = self.deep.next()?;
                }
            }
        }

        Ok(self.consume_smaller())
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        match self.last_consumed {
            Some(CursorSource::Shallow) => self.shallow.current(),
            Some(CursorSource::Deep) => self.deep.current(),
            None => Ok(None),
        }
    }

    fn reset(&mut self) {
        self.pending_shallow = None;
        self.pending_deep = None;
        self.last_consumed = None;
    }
}

// =============================================================================
// DatabaseStorageTrieCursor — unchanged single-table cursor
// =============================================================================

/// A cursor over the storage tries stored in the database.
#[derive(Debug)]
pub struct DatabaseStorageTrieCursor<C> {
    /// The underlying cursor.
    pub cursor: C,
    /// Hashed address used for cursor positioning.
    hashed_address: B256,
}

impl<C> DatabaseStorageTrieCursor<C> {
    /// Create a new storage trie cursor.
    pub const fn new(cursor: C, hashed_address: B256) -> Self {
        Self { cursor, hashed_address }
    }
}

impl<C> DatabaseStorageTrieCursor<C>
where
    C: DbCursorRO<tables::StoragesTrie>
        + DbCursorRW<tables::StoragesTrie>
        + DbDupCursorRO<tables::StoragesTrie>
        + DbDupCursorRW<tables::StoragesTrie>,
{
    /// Writes storage updates that are already sorted
    pub fn write_storage_trie_updates_sorted(
        &mut self,
        updates: &StorageTrieUpdatesSorted,
    ) -> Result<usize, DatabaseError> {
        if updates.is_deleted() && self.cursor.seek_exact(self.hashed_address)?.is_some() {
            self.cursor.delete_current_duplicates()?;
        }

        let mut num_entries = 0;
        for (nibbles, maybe_updated) in updates.storage_nodes.iter().filter(|(n, _)| !n.is_empty())
        {
            num_entries += 1;
            let nibbles = StoredNibblesSubKey(*nibbles);
            if self
                .cursor
                .seek_by_key_subkey(self.hashed_address, nibbles.clone())?
                .filter(|e| e.nibbles == nibbles)
                .is_some()
            {
                self.cursor.delete_current()?;
            }
            if let Some(node) = maybe_updated {
                self.cursor.upsert(
                    self.hashed_address,
                    &StorageTrieEntry { nibbles, node: node.clone() },
                )?;
            }
        }

        Ok(num_entries)
    }
}

impl<C> TrieCursor for DatabaseStorageTrieCursor<C>
where
    C: DbCursorRO<tables::StoragesTrie> + DbDupCursorRO<tables::StoragesTrie> + Send,
{
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self
            .cursor
            .seek_by_key_subkey(self.hashed_address, StoredNibblesSubKey(key))?
            .filter(|e| e.nibbles == StoredNibblesSubKey(key))
            .map(|value| (value.nibbles.0, value.node)))
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self
            .cursor
            .seek_by_key_subkey(self.hashed_address, StoredNibblesSubKey(key))?
            .map(|value| (value.nibbles.0, value.node)))
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.cursor.next_dup()?.map(|(_, v)| (v.nibbles.0, v.node)))
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        Ok(self.cursor.current()?.map(|(_, v)| v.nibbles.0))
    }

    fn reset(&mut self) {}
}

impl<C> TrieStorageCursor for DatabaseStorageTrieCursor<C>
where
    C: DbCursorRO<tables::StoragesTrie> + DbDupCursorRO<tables::StoragesTrie> + Send,
{
    fn set_hashed_address(&mut self, hashed_address: B256) {
        self.hashed_address = hashed_address;
    }
}

// =============================================================================
// ShallowStorageTrieCursor — typed for StoragesTrieShallow
// =============================================================================

#[derive(Debug)]
struct ShallowStorageTrieCursor<C> {
    cursor: C,
    hashed_address: B256,
}

impl<C> ShallowStorageTrieCursor<C>
where
    C: DbCursorRO<tables::StoragesTrieShallow>
        + DbDupCursorRO<tables::StoragesTrieShallow>
        + Send,
{
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self
            .cursor
            .seek_by_key_subkey(self.hashed_address, StoredNibblesSubKey(key))?
            .filter(|e| e.nibbles == StoredNibblesSubKey(key))
            .map(|v| (v.nibbles.0, v.node)))
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self
            .cursor
            .seek_by_key_subkey(self.hashed_address, StoredNibblesSubKey(key))?
            .map(|v| (v.nibbles.0, v.node)))
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        Ok(self.cursor.next_dup()?.map(|(_, v)| (v.nibbles.0, v.node)))
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        Ok(self.cursor.current()?.map(|(_, v)| v.nibbles.0))
    }

    fn set_hashed_address(&mut self, hashed_address: B256) {
        self.hashed_address = hashed_address;
    }
}

// =============================================================================
// SplitStorageTrieCursor — correct sorted-merge over two DupSort tables
// =============================================================================

/// Storage trie cursor that merges `StoragesTrieShallow` (shallow) and
/// `StoragesTrie` (deep) using the same two-way merge as
/// [`SplitAccountTrieCursor`].
#[derive(Debug)]
pub struct SplitStorageTrieCursor<CS, CD> {
    shallow: ShallowStorageTrieCursor<CS>,
    deep: DatabaseStorageTrieCursor<CD>,
    pending_shallow: Option<(Nibbles, BranchNodeCompact)>,
    pending_deep: Option<(Nibbles, BranchNodeCompact)>,
    last_consumed: Option<CursorSource>,
}

impl<CS, CD> SplitStorageTrieCursor<CS, CD> {
    pub fn new(shallow: CS, deep: CD, hashed_address: B256) -> Self {
        Self {
            shallow: ShallowStorageTrieCursor { cursor: shallow, hashed_address },
            deep: DatabaseStorageTrieCursor::new(deep, hashed_address),
            pending_shallow: None,
            pending_deep: None,
            last_consumed: None,
        }
    }
}

impl<CS, CD> SplitStorageTrieCursor<CS, CD>
where
    CS: DbCursorRO<tables::StoragesTrieShallow>
        + DbDupCursorRO<tables::StoragesTrieShallow>
        + Send,
    CD: DbCursorRO<tables::StoragesTrie>
        + DbDupCursorRO<tables::StoragesTrie>
        + Send,
{
    fn consume_smaller(&mut self) -> Option<(Nibbles, BranchNodeCompact)> {
        match (&self.pending_shallow, &self.pending_deep) {
            (Some((s, _)), Some((d, _))) => {
                if s <= d {
                    self.last_consumed = Some(CursorSource::Shallow);
                    self.pending_shallow.take()
                } else {
                    self.last_consumed = Some(CursorSource::Deep);
                    self.pending_deep.take()
                }
            }
            (Some(_), None) => {
                self.last_consumed = Some(CursorSource::Shallow);
                self.pending_shallow.take()
            }
            (None, Some(_)) => {
                self.last_consumed = Some(CursorSource::Deep);
                self.pending_deep.take()
            }
            (None, None) => {
                self.last_consumed = None;
                None
            }
        }
    }
}

impl<CS, CD> TrieCursor for SplitStorageTrieCursor<CS, CD>
where
    CS: DbCursorRO<tables::StoragesTrieShallow>
        + DbDupCursorRO<tables::StoragesTrieShallow>
        + Send,
    CD: DbCursorRO<tables::StoragesTrie>
        + DbDupCursorRO<tables::StoragesTrie>
        + Send,
{
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        self.pending_shallow = None;
        self.pending_deep = None;

        if key.len() <= SHALLOW_TRIE_DEPTH {
            self.last_consumed = Some(CursorSource::Shallow);
            self.shallow.seek_exact(key)
        } else {
            self.last_consumed = Some(CursorSource::Deep);
            self.deep.seek_exact(key)
        }
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        self.pending_shallow = self.shallow.seek(key.clone())?;
        self.pending_deep = self.deep.seek(key)?;
        Ok(self.consume_smaller())
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        match self.last_consumed {
            Some(CursorSource::Shallow) => {
                self.pending_shallow = self.shallow.next()?;
            }
            Some(CursorSource::Deep) => {
                self.pending_deep = self.deep.next()?;
            }
            None => {
                if self.pending_shallow.is_none() {
                    self.pending_shallow = self.shallow.next()?;
                }
                if self.pending_deep.is_none() {
                    self.pending_deep = self.deep.next()?;
                }
            }
        }
        Ok(self.consume_smaller())
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        match self.last_consumed {
            Some(CursorSource::Shallow) => self.shallow.current(),
            Some(CursorSource::Deep) => self.deep.current(),
            None => Ok(None),
        }
    }

    fn reset(&mut self) {
        self.pending_shallow = None;
        self.pending_deep = None;
        self.last_consumed = None;
    }
}

impl<CS, CD> TrieStorageCursor for SplitStorageTrieCursor<CS, CD>
where
    CS: DbCursorRO<tables::StoragesTrieShallow>
        + DbDupCursorRO<tables::StoragesTrieShallow>
        + Send,
    CD: DbCursorRO<tables::StoragesTrie>
        + DbDupCursorRO<tables::StoragesTrie>
        + Send,
{
    fn set_hashed_address(&mut self, hashed_address: B256) {
        self.shallow.set_hashed_address(hashed_address);
        self.deep.set_hashed_address(hashed_address);
        // Clear pending entries since we're switching to a different account.
        self.pending_shallow = None;
        self.pending_deep = None;
        self.last_consumed = None;
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::hex_literal::hex;
    use reth_db_api::{cursor::DbCursorRW, transaction::DbTxMut};
    use reth_provider::test_utils::create_test_provider_factory;

    fn test_node() -> BranchNodeCompact {
        BranchNodeCompact::new(
            0b0000_0010_0000_0001,
            0b0000_0010_0000_0001,
            0,
            Vec::default(),
            None,
        )
    }

    // ---- Original tests (preserved) ----

    #[test]
    fn test_account_trie_order() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();
        let mut cursor = provider.tx_ref().cursor_write::<tables::AccountsTrie>().unwrap();

        let data = vec![
            hex!("0303040e").to_vec(),
            hex!("030305").to_vec(),
            hex!("03030500").to_vec(),
            hex!("0303050a").to_vec(),
        ];

        for key in data.clone() {
            cursor
                .upsert(
                    key.into(),
                    &BranchNodeCompact::new(
                        0b0000_0010_0000_0001,
                        0b0000_0010_0000_0001,
                        0,
                        Vec::default(),
                        None,
                    ),
                )
                .unwrap();
        }

        let db_data = cursor.walk_range(..).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(db_data[0].0 .0.to_vec(), data[0]);
        assert_eq!(db_data[1].0 .0.to_vec(), data[1]);
        assert_eq!(db_data[2].0 .0.to_vec(), data[2]);
        assert_eq!(db_data[3].0 .0.to_vec(), data[3]);

        assert_eq!(
            cursor.seek(hex!("0303040f").to_vec().into()).unwrap().map(|(k, _)| k.0.to_vec()),
            Some(data[1].clone())
        );
    }

    #[test]
    fn test_storage_cursor_abstraction() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();
        let mut cursor = provider.tx_ref().cursor_dup_write::<tables::StoragesTrie>().unwrap();

        let hashed_address = B256::random();
        let key = StoredNibblesSubKey::from(vec![0x2, 0x3]);
        let value = BranchNodeCompact::new(1, 1, 1, vec![B256::random()], None);

        cursor
            .upsert(hashed_address, &StorageTrieEntry { nibbles: key.clone(), node: value.clone() })
            .unwrap();

        let mut cursor = DatabaseStorageTrieCursor::new(cursor, hashed_address);
        assert_eq!(cursor.seek(key.into()).unwrap().unwrap().1, value);
    }

    // ---- New split cursor tests ----

    #[test]
    fn split_cursor_seek_exact_routes_by_depth() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();
        let node = test_node();

        // Write shallow node (3 nibbles <= 5) to shallow table
        provider
            .tx_ref()
            .cursor_write::<tables::AccountsTrieShallow>()
            .unwrap()
            .upsert(StoredNibbles(Nibbles::from_nibbles([0x1, 0x2, 0x3])), &node)
            .unwrap();

        // Write deep node (8 nibbles > 5) to deep table
        provider
            .tx_ref()
            .cursor_write::<tables::AccountsTrie>()
            .unwrap()
            .upsert(
                StoredNibbles(Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8])),
                &node,
            )
            .unwrap();

        let f = DatabaseTrieCursorFactory::new(provider.tx_ref());
        let mut cursor = f.account_trie_cursor().unwrap();

        // seek_exact shallow
        let r = cursor.seek_exact(Nibbles::from_nibbles([0x1, 0x2, 0x3])).unwrap();
        assert!(r.is_some());
        assert_eq!(r.unwrap().0, Nibbles::from_nibbles([0x1, 0x2, 0x3]));

        // seek_exact deep
        let r = cursor
            .seek_exact(Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]))
            .unwrap();
        assert!(r.is_some());

        // seek_exact miss
        let r = cursor.seek_exact(Nibbles::from_nibbles([0xf, 0xf])).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn split_cursor_seek_then_next_produces_sorted_merge() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();
        let node = test_node();

        // Shallow table entries (nibble count <= 5)
        let mut sc = provider
            .tx_ref()
            .cursor_write::<tables::AccountsTrieShallow>()
            .unwrap();
        sc.upsert(StoredNibbles(Nibbles::from_nibbles([0x1])), &node).unwrap();
        sc.upsert(StoredNibbles(Nibbles::from_nibbles([0x3])), &node).unwrap();
        sc.upsert(StoredNibbles(Nibbles::from_nibbles([0x5])), &node).unwrap();
        drop(sc);

        // Deep table entries (nibble count > 5)
        let mut dc = provider
            .tx_ref()
            .cursor_write::<tables::AccountsTrie>()
            .unwrap();
        dc.upsert(
            StoredNibbles(Nibbles::from_nibbles([0x2, 0x0, 0x0, 0x0, 0x0, 0x0])),
            &node,
        )
        .unwrap();
        dc.upsert(
            StoredNibbles(Nibbles::from_nibbles([0x4, 0x0, 0x0, 0x0, 0x0, 0x0])),
            &node,
        )
        .unwrap();
        drop(dc);

        // Iterate with split cursor
        let f = DatabaseTrieCursorFactory::new(provider.tx_ref());
        let mut cursor = f.account_trie_cursor().unwrap();

        let first = cursor.seek(Nibbles::default()).unwrap();
        assert!(first.is_some());

        let mut results = vec![first.unwrap().0];
        while let Some((nibbles, _)) = cursor.next().unwrap() {
            results.push(nibbles);
        }

        // Must have all 5 entries in sorted order
        assert_eq!(results.len(), 5, "expected 5 entries, got {:?}", results);
        for w in results.windows(2) {
            assert!(w[0] < w[1], "not sorted: {:?} >= {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn split_cursor_boundary_depth_5_is_shallow() {
        // Nibbles of exactly length 5 should go to the shallow table
        let path = Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5]);
        assert!(path.len() <= SHALLOW_TRIE_DEPTH);
    }

    #[test]
    fn split_cursor_boundary_depth_6_is_deep() {
        // Nibbles of length 6 should go to the deep table
        let path = Nibbles::from_nibbles([0x1, 0x2, 0x3, 0x4, 0x5, 0x6]);
        assert!(path.len() > SHALLOW_TRIE_DEPTH);
    }

    #[test]
    fn split_cursor_empty_tables() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();

        let f = DatabaseTrieCursorFactory::new(provider.tx_ref());
        let mut cursor = f.account_trie_cursor().unwrap();

        assert!(cursor.seek(Nibbles::default()).unwrap().is_none());
        assert!(cursor.next().unwrap().is_none());
    }

    #[test]
    fn split_cursor_only_shallow_entries() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();
        let node = test_node();

        let mut sc = provider
            .tx_ref()
            .cursor_write::<tables::AccountsTrieShallow>()
            .unwrap();
        sc.upsert(StoredNibbles(Nibbles::from_nibbles([0x1])), &node).unwrap();
        sc.upsert(StoredNibbles(Nibbles::from_nibbles([0x2])), &node).unwrap();
        drop(sc);

        let f = DatabaseTrieCursorFactory::new(provider.tx_ref());
        let mut cursor = f.account_trie_cursor().unwrap();

        let first = cursor.seek(Nibbles::default()).unwrap();
        assert_eq!(first.unwrap().0, Nibbles::from_nibbles([0x1]));

        let second = cursor.next().unwrap();
        assert_eq!(second.unwrap().0, Nibbles::from_nibbles([0x2]));

        assert!(cursor.next().unwrap().is_none());
    }

    #[test]
    fn split_cursor_only_deep_entries() {
        let factory = create_test_provider_factory();
        let provider = factory.provider_rw().unwrap();
        let node = test_node();

        let mut dc = provider
            .tx_ref()
            .cursor_write::<tables::AccountsTrie>()
            .unwrap();
        dc.upsert(
            StoredNibbles(Nibbles::from_nibbles([0x1, 0x0, 0x0, 0x0, 0x0, 0x0])),
            &node,
        )
        .unwrap();
        dc.upsert(
            StoredNibbles(Nibbles::from_nibbles([0x2, 0x0, 0x0, 0x0, 0x0, 0x0])),
            &node,
        )
        .unwrap();
        drop(dc);

        let f = DatabaseTrieCursorFactory::new(provider.tx_ref());
        let mut cursor = f.account_trie_cursor().unwrap();

        let first = cursor.seek(Nibbles::default()).unwrap();
        assert!(first.is_some());

        let second = cursor.next().unwrap();
        assert!(second.is_some());

        assert!(cursor.next().unwrap().is_none());
    }
}
