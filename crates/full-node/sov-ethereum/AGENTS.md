# AGENTS.md - sov-ethereum

> See repository `AGENTS.md` for global rules. This file adds crate-specific guidance only.

## Mission

`sov-ethereum` is the full-node Ethereum RPC wrapper around `sov-evm`. It owns transport-facing behavior: tx submission lifecycle, log pagination, subscriptions, and unsupported-method stubs.

## Ownership Boundary

- Own here: `eth_sendRawTransaction*`, `realtime_sendRawTransaction`, `eth_getLogs`, `eth_getLogsWithCursor`, `eth_subscribe`/`eth_unsubscribe`, method stubs, wrapper error mapping.
- Do not own here: canonical EVM state query logic (`eth_getBalance`, `eth_call`, `eth_estimateGas`, receipts, block assembly). Those belong in `crates/module-system/module-implementations/sov-evm`.

## Shared RPC Semantics (Intentional)

These are by-design repo semantics. Do not flag as bugs unless a concrete tooling workflow breaks.

- `latest` and `pending` resolve to the same pending head view for block/tag resolution.
- Pending/soft-confirmed data may appear in block/tx/receipt/log responses.
- `safe` and `finalized` both resolve to the latest finalized block.
- `eth_getTransactionCount(..., "latest")` may include pending-inclusive nonce effects.
- `eth_getLogs` with `toBlock: "latest"` may include pending-head logs.
- `newHeads` may surface synthetic/pending-head style headers.
- `eth_blockNumber` returns latest sealed block number, not synthetic pending height.
- `eth_gasPrice` returns current `block_env.basefee`.
- `eth_maxPriorityFeePerGas` returns `0`.
- `eth_call` accepts `state_overrides`/`block_overrides` but currently ignores them.
- EIP-1898 `requireCanonical` is accepted but effectively a no-op in no-reorg semantics.

## Wrapper Hotspots

| Path | Why it matters |
| --- | --- |
| `src/lib.rs` | RPC registration, unsupported method stubs, wrapper error-code helpers |
| `src/handlers/mod.rs` | Raw tx submission, sync timeout behavior, local signing/send flow |
| `src/handlers/get_logs.rs` | Standard logs endpoint and response-size gate |
| `src/handlers/get_logs/service.rs` | Filter execution, block range/hash routing, cursor behavior |
| `src/handlers/get_logs/cursor.rs` | Cursor encoding/decoding and paging correctness |
| `src/handlers/subscribe/*.rs` | Subscription params validation, streaming, dedup/watermark behavior |

## Error Code Policy

Use shared helpers in `src/lib.rs`; do not introduce ad-hoc codes in handlers.

| Meaning | Code |
| --- | --- |
| Method not supported | `-32004` |
| Limit exceeded | `-32005` |
| Tx rejected | `-32003` |
| Resource not found | `-32001` |
| Invalid params | `-32602` |
| Sync timeout (`eth_sendRawTransactionSync`) | `4` |

## Failure Pattern Matrix (PR Lessons)

| Pattern | Repeated in PR(s) | Typical root cause | What to verify before merge |
| --- | --- | --- | --- |
| Error-code drift | `#2378`, `#2459` | Handler-specific error conversion bypasses wrapper helpers | Invalid params / rejected tx / not-found / limit paths map to expected codes |
| Registration or stub gaps | `#2381` | New/unsupported methods not registered or not stubbed | Every unsupported method returns wrapper “not supported” code instead of “method not found” |
| Block-id and pending assumption mismatch | `#2379`, `#2391` | Wrapper assumes L1 selector semantics not matching module semantics | Submission/local flows calling `sov-evm` use `BlockId` intentionally and consistently |
| Logs pagination inconsistency | `#2387` | `eth_getLogs` and cursor variant diverge in range/size behavior | Large responses return limit error plus cursor path; cursor pagination remains stable |
| Nonce/receipt flow surprises | `#2395`, `#2458` | Submission paths do not align with pending semantics | Wallet lifecycle (`send -> lookup tx -> receipt`) is coherent in pending and sealed states |
| Estimation/submission mismatch | `#2459` | Local `eth_sendTransaction` mutates request fields inconsistently | Nonce/chain-id/gas defaults are explicit; estimation errors propagate cleanly |
| Fee-context confusion at wrapper boundary | `#2462`, `#2463` | Wrapper assumes fee values independent of module context | Wrapper does not override module-computed fee/receipt semantics |

## Wiring Rules

1. If a method is a state query, implement it in `sov-evm` first.
2. If a method is unsupported, add or keep an explicit stub in `src/lib.rs`.
3. Use wrapper helpers (`rpc_invalid_params`, `rpc_tx_rejected`, `rpc_limit_exceeded`, `rpc_resource_not_found`) for handler errors.
4. Keep local-only methods behind `local` feature gates.
5. Do not duplicate EVM business logic in wrapper handlers.

## Tx Submission Guardrails

1. Keep parse -> authenticate -> sequencer accept ordering stable.
2. Preserve explicit max timeout behavior for `eth_sendRawTransactionSync`.
3. Ensure tx-type rejection remains clear for unsupported tx types.
4. In local signing flow, keep nonce/chain-id/gas filling explicit and deterministic.

## Logs and Subscription Guardrails

1. `eth_getLogs` must enforce response-size limits and direct users to cursor path when needed.
2. Cursor path must preserve deterministic forward progress and strict cursor validation.
3. Subscription validation must remain explicit for supported kinds (`logs`, `newHeads`) and parameter constraints.
4. Keep shutdown and stream termination behavior graceful and deterministic.

## Pre-Merge Checklist

1. If touching error mapping, test each helper code path at least once.
2. If touching registration, verify unsupported methods still return stubbed code.
3. If touching submission, test raw send, sync send timeout, and receipt retrieval flow.
4. If touching logs, test both no-cursor and cursor pagination paths with limits.
5. If touching subscriptions, test `logs` and `newHeads` parameter validation and stream behavior.
6. If touching local signing flow, verify nonce/chain-id/gas defaults and failure mapping.

## Minimal Audit Notes (P0/P1)

Prioritize static checks that map to real client breakage:

1. Wallet flow integrity: send, estimate, poll tx, poll receipt.
2. SDK compatibility: ethers/viem/web3 parsing and retry behavior under wrapper errors.
3. Logs/indexer safety: bounded responses, stable cursor continuation, predictable not-found behavior.

## Fast Commands

```bash
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-ethereum
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_logs
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_subscribe
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_ws_watch
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm_tx
```
