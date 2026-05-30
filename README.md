## 0xMars42

Rust systems engineer working on EVM internals, MEV, and low-latency on-chain
infrastructure. I build read-only tooling that decodes and reasons about live
chain activity, and I contribute to the Rust Ethereum stack.

Open to **full-remote** roles in blockchain infrastructure, MEV, and crypto-quant.

### Selected work

**[eth-mempool-watcher](https://github.com/0xMars42/eth-mempool-watcher)**
Real-time Ethereum L1 mempool monitor. Subscribes to full pending transactions,
decodes DEX swap calldata across 12 selectors (Uniswap V2/V3, Universal Router,
1inch), and runs a detect to track to validate pipeline that flags sniper and
sandwich patterns, then confirms them against on-chain receipts. Rust, alloy,
tokio. 22 tests, CI.

**[base-arb-scanner](https://github.com/0xMars42/base-arb-scanner)**
Cross-DEX arbitrage scanner for Base. WebSocket `newHeads`, three pools read
atomically through Multicall3, two-stage spot then Quoter filtering with net P&L
after dynamic gas and fees. Read-only, zero capital. Rust, alloy, tokio. 25
tests, CI.

**[evm-opcode-bench](https://github.com/0xMars42/evm-opcode-bench)**
Per-opcode EVM microbenchmark on revm. Measures real CPU nanoseconds per opcode,
computes gas/ns efficiency, and surfaces mispriced opcodes.

### Open source

Contributing to **[Lighthouse](https://github.com/sigp/lighthouse)**, the Rust
Ethereum consensus client:

- [#9373](https://github.com/sigp/lighthouse/pull/9373) instrument the fork
  choice lock with hold-time metrics and backtraces (#8147)
- [#9376](https://github.com/sigp/lighthouse/pull/9376) gate custody backfill
  peer selection on custody columns, a PeerDAS liveness fix with a regression
  test (#8308)

### Stack

Rust, C, C++, x86 assembly. EVM internals, alloy, tokio, foundry, revm. Async
networking and low-latency data paths.

### Contact

[GitHub](https://github.com/0xMars42) &middot; [X](https://x.com/0xMars42)
