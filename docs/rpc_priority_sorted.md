# RPC Priority (Sorted for L1-Parity Tests)

## Goals and scope
- L1-indistinguishable RPC behavior for end users; semantic correctness first, schema correctness second.
- Primary focus: "latest/head" vs "pending" behavior for methods that accept block selectors/tags.
- Scope: tests + test harness only; do not fix production bugs here.
- Out of scope: invalid-param / error-shape conformance tests.

## Prioritization criteria (context)
- Based on common call patterns from wallets, dapp SDKs, explorers, and dev tooling (no telemetry).
- Ranked by UX impact, dapp dependency, block-tag sensitivity, and test ROI.

## Inputs
- Implemented endpoint list and notes: [docs/rpc_inventory.md]
- Implementations: [crates/module-system/module-implementations/sov-evm], [crates/full-node/sov-ethereum]
- Existing tests (quick scan, not exhaustive): [examples/demo-rollup/tests/evm]

## Current coverage snapshot (quick scan, not exhaustive)
- Block queries and receipts: `eth_getBlockByNumber`, `eth_getBlockByHash`, `eth_getBlockReceipts` cover latest/pending tags in baseline tests [examples/demo-rollup/tests/evm/evm_rpc.rs:32].
- Logs: extensive `eth_getLogs` and `eth_getLogsWithCursor` coverage including pending/latest/safe/finalized tags [examples/demo-rollup/tests/evm/evm_logs.rs:441].
- Balances + nonces: `eth_getBalance` and `eth_getTransactionCount` used without explicit block tags [examples/demo-rollup/tests/evm/evm_tx.rs:52] [examples/demo-rollup/tests/evm/evm_balances.rs:78].
- Calls + estimation: `eth_call` and `eth_estimateGas` used but not block-tagged [examples/demo-rollup/tests/evm/evm_tx.rs:93] [examples/demo-rollup/tests/evm/evm_gas_estimation.rs:15].
- Code + storage: `eth_getCode` and `eth_getStorageAt` tested at default tag only [examples/demo-rollup/tests/evm/evm_test_helper.rs:132] [examples/demo-rollup/tests/evm/evm_test_helper.rs:153].
- Subscriptions: `eth_subscribe`/`eth_unsubscribe` coverage for logs/newHeads [examples/demo-rollup/tests/evm/evm_subscribe.rs].

## Priority tiers (ordered within each tier)

### P0 - Block-tag and pending semantics (highest ROI, ranked)
1. `eth_getBlockByNumber` + `eth_getBlockByHash` - authoritative latest/pending semantics; tx hashes vs objects; EIP-1898 blockHash.
2. `eth_blockNumber` - head monotonicity and alignment with `eth_getBlockByNumber("latest")`.
3. `eth_getTransactionCount` - pending vs latest nonce; EIP-1898 blockHash.
4. `eth_getLogs` - range/tag semantics; `blockHash` filter; consistency with receipts.
5. `eth_getTransactionReceipt` - null while pending; stable once sealed.
6. `eth_getTransactionByHash` - pending vs mined fields (`blockNumber`, `blockHash`).
7. `eth_call` - block-tagged state reads; no pending bleed into latest.
8. `eth_estimateGas` - tag-sensitive estimation; EIP-1898 blockHash.
9. `eth_getBalance` - state at tags; EIP-1898 blockHash.
10. `eth_subscribe` / `eth_unsubscribe` (logs, newHeads)
11. `eth_sendRawTransactionSync` (custom)
12. `realtime_sendRawTransaction` (custom)
13. `debug_traceTransaction`
14. ~~`eth_sendRawTransaction` - submission path required for all lifecycle tests.~~ We are fine

### P1 - Wallet connect, fees, and contract inspection
1. `eth_chainId`
2. `net_version`
3. `eth_gasPrice`
4. `eth_maxPriorityFeePerGas`
5. `eth_feeHistory`
6. `eth_getCode`
7. `eth_getStorageAt`
8. `eth_getBlockReceipts`
9. `eth_getBlockTransactionCountByNumber`
10. `eth_getBlockTransactionCountByHash`
11. `web3_clientVersion`
12. `web3_sha3`
13. `net_listening`
14. `eth_accounts` (local-only)
15. `eth_sendTransaction` (local-only)

### P2 - Debugging and Sovereign-specific RPC
- `debug_traceBlockByNumber`
- `eth_getLogsWithCursor` (custom)


### P3 - Unsupported methods
- Methods listed as "Method not supported" in [docs/rpc_inventory.md].

## Quick-start (top 10)
- Use P0 items 1-10 in order; ensure `eth_getBlockByNumber` covers both transaction hashes and full objects.

## Block selector coverage (apply to P0/P1 state methods)
- Tags: `earliest`, `latest`, `pending`, `safe`, `finalized`, `Number(n)`.
- EIP-1898 `blockHash` object with `requireCanonical` true/false for:
  `eth_getBalance`, `eth_getTransactionCount`, `eth_getCode`, `eth_getStorageAt`,
  `eth_call`, `eth_estimateGas`, `eth_getBlockByNumber`, `eth_getBlockReceipts`,
  `eth_getBlockTransactionCountByNumber`, and `eth_getLogs` with `filter.blockHash`.
- Scenarios: empty state, after 1 block, after several blocks, with pending txs,
  and unknown blockHash (assert error/null only, not error shape).
- For canonical hashes, expect `requireCanonical` true/false to return identical results.
- Minimal blockHash fixture: record N0/H0, deploy (N1/H1), set value (N2/H2),
  emit logs (N3/H3), transfer (N4/H4), then pause sequencer for assertions.

## Invariants to assert (semantic > schema)
1. `eth_blockNumber` never decreases within a session.
2. `eth_getBlockByNumber("latest").number == eth_blockNumber`.
3. `eth_getBlockByNumber("pending").number == eth_blockNumber + 1` and `hash == null`.
4. Data for a sealed block never changes.
5. Receipts/logs/transactions agree on block hash/number once sealed.

## Likely L1 divergences to capture via tests (do not fix here)
1. `latest` is treated as `pending` in block resolution [crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs:242], and existing tests assert `latest == pending` [examples/demo-rollup/tests/evm/evm_tx.rs:47] with `hash == 0x0` [examples/demo-rollup/tests/evm/evm_tx.rs:48].  
   Minimal repro: pause sequencer, call `eth_getBlockByNumber("latest")` and `eth_getBlockByNumber("pending")`; expect different blocks per L1, but current behavior returns the same pending block.
2. `eth_getTransactionCount` includes pending txs for `latest` [crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs:168].  
   Minimal repro: pause sequencer, send tx, compare nonce for `latest` vs `pending`; L1 expects `latest` to ignore pending.
3. `eth_getTransactionReceipt` and `eth_getTransactionByHash` return objects with `blockNumber` set before sealing [examples/demo-rollup/tests/evm/evm_soft_conf.rs:40].  
   Minimal repro: pause sequencer, send tx, query receipt/tx; L1 expects `null` until mined.
4. `safe`/`finalized` tags map to head block [crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs:244].  
   Minimal repro: run with `finalization_blocks > 0`, produce blocks, and assert `finalized` < `latest`.
5. `eth_gasPrice` and `eth_maxPriorityFeePerGas` always return 0 [crates/full-node/sov-ethereum/src/lib.rs:122] [crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs:414].  
   Minimal repro: call both endpoints; L1 typically returns non-zero.
6. `eth_call` ignores `state_overrides` and `block_overrides` [crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs:268].  
   Minimal repro: pass overrides that would change state; expect no effect.

## Anti-flakiness
- Use explicit block production controls (pause/resume).
- Avoid wall-clock timing assertions; prefer invariants.
- Use deterministic transaction ordering.
