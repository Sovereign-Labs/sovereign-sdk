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

## Test cases (comprehensive)

### 1) Baseline nonce at head (no pending)
Setup:
- Start rollup, wait for at least 1 block, pause batches.
Steps:
- Query `eth_getTransactionCount` for sender at block `latest`.
Expected:
- Nonce is a concrete value (typically 0 for a fresh account).
- Nonce is stable across repeated calls without txs.

### 2) Pending vs latest nonce (rollup behavior)
Setup:
- Start rollup, wait for at least 1 block, pause batches.
Steps:
Expected (rollup):
- `latest` nonce equals `pending` nonce at all times.
- After sending one tx, both tags return N0 + 1.

### 3) Block number selector (sealed block)
Setup:
- Start rollup, wait for at least 1 block, pause batches.
Steps:
- Capture head block number H (sealed).
- Read nonce at block number H.
- Send a tx while paused (no new sealed block).
- Read nonce again at block number H.
Expected:
- Nonce for block H is stable and unchanged by pending txs.

### 4) EIP-1898 blockHash selector (canonical block)
Setup:
- Start rollup, wait for at least 1 block, pause batches.
Steps:
- Fetch block H by number and extract its hash HH.
- Query `eth_getTransactionCount` with block selector:
  `{"blockHash": HH, "requireCanonical": true}`.
- Query again with `{"blockHash": HH, "requireCanonical": false}`.
Expected:
- Both queries return the same nonce as the block-number query for H.

### 5) Unknown blockHash selector
Setup:
- Start rollup, pause batches.
Steps:
- Query `eth_getTransactionCount` with a random block hash.
Expected:
- L1 returns an error for unknown block hash; do not assert error shape.
Notes:
- If current behavior returns `null` or error, document the observed behavior.

### 6) Multiple senders, independent nonces
Setup:
- Start rollup, wait for at least 1 block, pause batches.
Steps:
- Use two signers (primary and secondary).
- Send one tx from primary, two txs from secondary (paused).
- Read `latest`/`pending` for both addresses.
Expected:
- Primary nonce increments by 1, secondary nonce increments by 2.
- Nonces are independent and do not affect each other.

### 7) Nonce monotonicity across sequential tx submissions
Setup:
- Start rollup, pause batches.
Steps:
- Read nonce N0 at `latest`.
- Send three txs in order from the same sender.
- Read nonce after each submission (paused).
Expected:
- Nonce increments by exactly 1 per submitted tx.
- No gaps or duplicate values in the sequence.

### 8) Reverted tx still consumes nonce (mined)
Setup:
- Deploy a contract with a method that reverts.
- Resume batches to mine a block.
Steps:
- Read nonce N0 at sealed head.
- Send a tx that will revert (then resume and mine).
- Read nonce at the new sealed head.
Expected:
- Nonce at the sealed head increases by 1 despite revert.
- Receipt status indicates failure, but nonce still advanced.

### 9) Pending block contents vs nonce
Setup:
- Pause batches and submit multiple txs from the same sender.
Steps:
- Fetch `eth_getBlockByNumber("pending")` and count sender txs.
- Query `eth_getTransactionCount` for sender.
Expected:
- Nonce equals sealed nonce + count of pending sender txs.
- Count matches the number of sender txs in the pending block.

### 10) Cross-endpoint consistency with receipts
Setup:
- Resume batches to mine a block containing N txs from sender.
Steps:
- Get receipts for those txs and collect block number H.
- Query `eth_getTransactionCount(address, H)` and `H-1`.
Expected:
- Nonce at H equals nonce at H-1 plus N.
- Receipts for block H are consistent with the nonce increment.

### 11) Safe/finalized tags with finalization enabled
Setup:
- Start rollup with finalization_blocks > 0.
- Produce multiple blocks.
Steps:
- Query nonce at `safe`, `finalized`, and `latest`.
Expected:
- Nonce at `safe`/`finalized` reflects the corresponding sealed block.
- Nonces should be <= `latest` nonce.

### 12) Earliest tag
Setup:
- Start rollup, pause batches.
Steps:
- Query nonce at `earliest`.
Expected:
- Nonce matches the genesis state for that account (typically 0).

### 13) Explicit block number boundary
Setup:
- Produce blocks up to height H and pause.
Steps:
- Query nonce at H and H-1.
Expected:
- Nonce at H is >= nonce at H-1.
- If no txs from sender were mined in H, the values are equal.

## Cross-endpoint invariants (optional)
- If `eth_getBlockByNumber(H)` returns a sealed block, then
  `eth_getTransactionCount(address, H)` must remain constant for that H.
- `pending` nonce should be equal to `latest` nonce in this rollup.

## Contract deployment requirements
- None. A simple ETH transfer tx is sufficient to increment nonce.
