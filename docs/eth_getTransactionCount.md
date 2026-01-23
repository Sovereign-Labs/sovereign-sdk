# eth_getTransactionCount test cases (human-readable)

Scope: Sovereign SDK EVM JSON-RPC parity tests for `eth_getTransactionCount`.
Priority: Semantic correctness > schema correctness. Error-shape tests are out of scope.

## Spec summary (target behavior in this rollup)
- Method: `eth_getTransactionCount(address, block)` returns the account nonce at the
  given block selector.
- Default block selector is `latest` if omitted.
- Rollup design: `latest` and `pending` are treated as the same tag, so they should
  return identical nonces in this system.
- EIP-1898 `{"blockHash": ..., "requireCanonical": ...}` selects a specific block by hash.
  `requireCanonical` should not change the result for canonical hashes.

## Deterministic setup (test harness)
- Use `setup_with_simple_storage(finalization_blocks, EVM_EXTENSION)`.
- Use `pause_preferred_batches()` to freeze sealing and keep pending state stable.
- Use `wait_for_next_blocks(1)` to advance head deterministically.
- Use `SimpleStorageClient` to send txs and query RPCs.

## Test cases (quality-first, state-driven)

### Core fixture (deterministic, reused)
Goal:
- Establish a precise, reproducible nonce timeline for two accounts.
Setup:
- Two signers: A (primary), B (secondary).
- Produce one sealed block, then pause batches to hold a pending block.
Sequence:
1) Seal block H0 with no A/B txs (record nonce A0, B0 at H0).
2) Pause batches (pending exists).
3) Submit txs in order: A1, B1, B2, A2 (all pending).
4) Resume batches and seal block H1 containing those txs.
5) Pause batches again for pending-state assertions.
Derived expectations:
- Pending/latest nonce after step 3: A = A0 + 2, B = B0 + 2.
- Sealed nonce at H1: A = A0 + 2, B = B0 + 2.

### 1) Default selector and rollup tag semantics
Steps:
- Query `eth_getTransactionCount(A)` with no block selector.
- Query `eth_getTransactionCount(A, "latest")` and `eth_getTransactionCount(A, "pending")`.
Expected:
- Default equals `latest`.
- `latest` equals `pending` for A and B at all times in this rollup.

### 2) Sealed history is immutable
Steps:
- Read nonce at H0.
- While paused, submit pending txs (A1, B1, B2, A2).
- Read nonce again at H0.
Expected:
- Nonce at H0 is unchanged by pending txs.

### 3) Per-address isolation with interleaved submissions
Steps:
- Use the core fixture sequence (A1, B1, B2, A2).
- Query nonces at pending/latest and at sealed H1.
Expected:
- A nonce increases by 2, B nonce increases by 2.
- Ordering does not leak across accounts.

### 4) Pending block contents ↔ nonce consistency
Steps:
- While paused after step 3, fetch `eth_getBlockByNumber("pending")`.
- Count pending txs from A and from B in the pending block.
Expected:
- For each address: nonce == sealed_nonce(H0) + pending_tx_count_for_address.

### 5) Sealed block receipts ↔ nonce delta
Steps:
- After sealing H1, collect receipts for txs in H1.
- Count txs from A and B included in H1.
- Query nonce at H1 and H0 for A and B.
Expected:
- nonce(H1) == nonce(H0) + count_in_H1 for each address.

### 6) Reverted tx still consumes nonce (sealed)
Steps:
- Deploy a contract with a reverting method.
- Submit a reverting tx from A and seal a block.
- Query nonce at H_before and H_after.
Expected:
- Nonce increases by 1 despite revert; receipt status indicates failure.

### 7) Sequential submission monotonicity (single sender)
Steps:
- With batches paused, submit three txs from A in order.
- Query nonce after each submission.
Expected:
- Nonce increases by exactly 1 per submission (no gaps).

### 8) EIP-1898 blockHash selector (canonical)
Steps:
- Fetch sealed block H1 and hash HH1.
- Query with `{"blockHash": HH1, "requireCanonical": true}` and false.
Expected:
- Both results equal `eth_getTransactionCount(..., H1)` for A and B.

### 9) Unknown blockHash selector
Steps:
- Query with a random block hash.
Expected:
- Error or null is acceptable; do not assert error shape.
Notes:
- Record observed behavior for stability across releases.

### 10) Tags: `earliest`, `safe`, `finalized`
Steps:
- For finalization_blocks = 0: query `earliest`, `safe`, `finalized`, `latest`.
- For finalization_blocks > 0: produce multiple blocks and query those tags again.
Expected:
- `earliest` reflects genesis nonce.
- `safe`/`finalized` map to their corresponding sealed blocks and are <= `latest`.

## Cross-endpoint invariants (optional)
- If `eth_getBlockByNumber(H)` returns a sealed block, then
  `eth_getTransactionCount(address, H)` must remain constant for that H.
- `pending` nonce should be equal to `latest` nonce in this rollup.

## Contract deployment requirements
- None. A simple ETH transfer tx is sufficient to increment nonce.
