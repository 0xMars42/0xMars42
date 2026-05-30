## 0xMars42

Rust systems engineer working on EVM internals, MEV, and low-latency on-chain
infrastructure. I build read-only tooling that decodes and reasons about live
chain activity, and I contribute to the core Rust Ethereum stack (reth, foundry,
revm, Lighthouse), including merged PRs in Paradigm's reth and foundry.

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

Contributions across the Rust Ethereum stack:

- **[reth](https://github.com/paradigmxyz/reth)** (Paradigm) — merged [#22168](https://github.com/paradigmxyz/reth/pull/22168) (fix an ExEx notification-channel stall during backfill); further PRs on engine prewarming, RPC trace timeouts, and MDBX storage perf.
- **[foundry](https://github.com/foundry-rs/foundry)** (Paradigm) — merged [#13389](https://github.com/foundry-rs/foundry/pull/13389) (skip redundant config remapping detection); async trace identification and faster anvil block mining.
- **[revm](https://github.com/bluealloy/revm)** (bluealloy) & **[alloy](https://github.com/alloy-rs)** — storage / allocation perf, unsafe-bytecode debug assertions, smaller proof-verification errors.
- **[rbuilder](https://github.com/flashbots/rbuilder)** (Flashbots) — track EXTCODEHASH / EXTCODESIZE / EXTCODECOPY in the state-access inspector.
- **[Lighthouse](https://github.com/sigp/lighthouse)** (Sigma Prime) — [#9373](https://github.com/sigp/lighthouse/pull/9373) fork-choice lock instrumentation, [#9376](https://github.com/sigp/lighthouse/pull/9376) PeerDAS custody-backfill liveness fix.

### Stack

Rust, C, C++, x86 assembly. EVM internals, alloy, tokio, foundry, revm. Async
networking and low-latency data paths.

### Contact

[GitHub](https://github.com/0xMars42) &middot; [X](https://x.com/0xMars42)
