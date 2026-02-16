/// The maximum size of RLP encoded trie account in bytes.
/// 2 (header) + 4 * 1 (field lens) + 8 (nonce) + 32 * 3 (balance, storage root, code hash)
pub const TRIE_ACCOUNT_RLP_MAX_SIZE: usize = 110;

/// Maximum nibble path length for nodes stored in the shallow trie tables.
///
/// Nodes with `path.len() <= SHALLOW_TRIE_DEPTH` go to `AccountsTrieShallow` /
/// `StoragesTrieShallow`. Based on Nethermind's analysis: shallow nodes (~648 MB,
/// 0.5% of total trie) are accessed on nearly every state read, making them
/// ideal candidates for separate caching / column family allocation.
///
/// See: <https://github.com/paradigmxyz/reth/issues/21183>
/// See: <https://github.com/NethermindEth/nethermind/pull/9854>
pub const SHALLOW_TRIE_DEPTH: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrieAccount;
    use alloy_primitives::{B256, U256};
    use alloy_rlp::Encodable;

    #[test]
    fn account_rlp_max_size() {
        let account = TrieAccount {
            nonce: u64::MAX,
            balance: U256::MAX,
            storage_root: B256::from_slice(&[u8::MAX; 32]),
            code_hash: B256::from_slice(&[u8::MAX; 32]),
        };
        let mut encoded = Vec::new();
        account.encode(&mut encoded);
        assert_eq!(encoded.len(), TRIE_ACCOUNT_RLP_MAX_SIZE);
    }

    #[test]
    fn shallow_depth_boundary() {
        assert_eq!(SHALLOW_TRIE_DEPTH, 5);
    }
}
