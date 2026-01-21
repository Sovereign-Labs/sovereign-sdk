# RPC Priority (Ethereum Compatibility)

Assumptions (explicit):
- This prioritization is based on typical call patterns from ethers.js, viem, wagmi, Foundry, Hardhat, and common block explorers, not on telemetry.
- The list is scoped to methods currently implemented in this repo (see `docs/rpc_inventory.md`).
- Primary focus is semantic correctness for block tags (`latest`, `pending`, `safe`, `finalized`) and block selectors.

## Tiered priorities

P0 (core wallet/dapp UX and transaction lifecycle; highest ROI for correctness):
- `eth_getBlockByNumber`
- `eth_getBlockByHash`
- `eth_blockNumber`
- `eth_getBalance`
- `eth_getTransactionCount`
- `eth_sendRawTransaction`
- `eth_getTransactionByHash`
- `eth_getTransactionReceipt`
- `eth_getLogs`
- `eth_call`
- `eth_estimateGas`

P1 (important but secondary; gas UX, state inspection, and richer explorers):
- `eth_chainId`
- `net_version`
- `eth_gasPrice`
- `eth_maxPriorityFeePerGas`
- `eth_feeHistory`
- `eth_getCode`
- `eth_getStorageAt`
- `eth_getBlockReceipts`
- `eth_getBlockTransactionCountByNumber`
- `eth_getBlockTransactionCountByHash`
- `eth_subscribe` / `eth_unsubscribe` (logs, newHeads)
- `eth_getLogsWithCursor` (custom pagination)
- `eth_sendRawTransactionSync` (custom)
- `realtime_sendRawTransaction` (custom)
- `web3_clientVersion`
- `web3_sha3`
- `net_listening`
- `eth_accounts` (local-only)
- `eth_sendTransaction` (local-only)

P2 (dev tooling / debugging):
- `debug_traceBlockByNumber`
- `debug_traceTransaction`

P3 (explicitly unsupported today):
- Methods listed as "Method not supported" in `docs/rpc_inventory.md`.

## Top 10 to test first (ranked, with rationale)

1. `eth_getBlockByNumber` - Primary entry point for "latest" vs "pending" semantics and header correctness.
2. `eth_getTransactionCount` - Wallet nonce management; must reflect pending txs for `pending`.
3. `eth_getBalance` - Core wallet UX; needs correct block-tagged state.
4. `eth_getTransactionReceipt` - Transaction lifecycle; pending vs mined behavior and block linkage.
5. `eth_getTransactionByHash` - Pending visibility and consistency with receipts.
6. `eth_getLogs` - Block-tagged range semantics; essential for explorers and dapp event indexing.
7. `eth_call` - Read-only execution correctness at tags and numbers.
8. `eth_estimateGas` - Transaction UX; block-tagged estimation impacts signing flows.
9. `eth_sendRawTransaction` - Submission path must accept and surface txs used in the above tests.
10. `eth_getBlockByHash` - Consistency check for block references from receipts/logs.
