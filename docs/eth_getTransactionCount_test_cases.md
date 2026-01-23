# eth_getTransactionCount test cases

## Summary

- Returns the nonce (number of transactions sent from an address) at a given block.
- Default block selector is `latest` if omitted.
- Rollup design: `latest` and `pending` are treated identically (both return pending state).
- EIP-1898 `{"blockHash": ..., "requireCanonical": ...}` selects by hash.

## Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `address` | `DATA`, 20 bytes | The address to query |
| `block` | `QUANTITY\|TAG\|Object` | Block number (hex), tag, or EIP-1898 object |

Valid tags: `earliest`, `latest`, `pending`, `safe`, `finalized`

## Curl examples

### Basic: get nonce at latest block
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getTransactionCount",
    "params": ["0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "latest"],
    "id": 1
  }'
```

### Query at specific block number
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getTransactionCount",
    "params": ["0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266", "0x5"],
    "id": 1
  }'
```

### Query by block hash (EIP-1898)
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getTransactionCount",
    "params": [
      "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
      {"blockHash": "0x7c5a35e9cb3e8ae0e221ab470abae9d446c3a5626ce6689fc777dcffcab52c70", "requireCanonical": true}
    ],
    "id": 1
  }'
```

### Example response
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": "0x5"
}
```

## Rollup-specific semantics

- `latest` and `pending` resolve to pending state (identical behavior).
- `safe` and `finalized` map to the sealed head.
- Non-existent addresses return `0x0` (not an error).

## Test harness setup

```rust
// Two signers for isolation testing
let (rollup, client_a, _) = setup_with_simple_storage(finalization_blocks, EVM_EXTENSION).await;
let client_b = create_simple_storage_client(rollup.http_addr, SECONDARY_SENDER_PRIV_KEY).await;

// Control block production
rollup.pause_preferred_batches().await;
rollup.resume_preferred_batches().await;
rollup.wait_for_next_blocks(1).await;
```

## Core fixture (deterministic, reused)

Setup: Two signers (A, B), produce sealed block H0, then pause.

Sequence:
1. Seal block H0 with no A/B txs (record nonce A0, B0)
2. Pause batches
3. Submit txs: A1, B1, B2, A2 (interleaved)
4. Resume and seal block H1
5. Pause again

Expected: A nonce = A0 + 2, B nonce = B0 + 2.

## Test cases

Priority: P0 = must-have, P1 = high value, P2 = medium, P3 = optional.

### Block selector semantics

| TC | Pri | Name | Description |
|----|-----|------|-------------|
| 01 | P0 | `default_selector_equals_latest` | Omitted block param equals `"latest"` |
| 02 | P0 | `latest_equals_pending` | Both return same nonce (rollup design) |
| 03 | P1 | `earliest_returns_genesis_nonce` | `"earliest"` returns 0 for fresh addresses |
| 04 | P1 | `safe_finalized_semantics` | Both map to sealed head |
| 05 | P1 | `specific_block_number` | Query nonce at hex block number |
| 06 | P0 | `sealed_history_immutable` | Nonce at H0 unchanged by pending txs |

### EIP-1898 block hash

| TC | Pri | Name | Description |
|----|-----|------|-------------|
| 07 | P1 | `blockhash_equals_block_number` | Same nonce by hash or number |
| 08 | P1 | `require_canonical_variants` | `true`/`false` return same result |
| 09 | P1 | `unknown_blockhash_errors` | Random hash returns error |

### Nonce increment rules

| TC | Pri | Name | Description |
|----|-----|------|-------------|
| 10 | P0 | `initial_nonce_is_zero` | Fresh address has nonce 0 |
| 11 | P0 | `increments_by_one` | Each tx increases nonce by 1 |
| 12 | P0 | `reverted_tx_consumes_nonce` | Revert still increments nonce |
| 13 | P1 | `sequential_monotonicity` | No gaps in nonce sequence |
| 14 | P1 | `per_address_isolation` | A/B nonces independent |

### Cross-validation

| TC | Pri | Name | Description |
|----|-----|------|-------------|
| 15 | P1 | `pending_block_nonce_consistency` | nonce == sealed + pending_tx_count |
| 16 | P1 | `sealed_receipts_nonce_delta` | nonce(H1) == nonce(H0) + tx_count_in_H1 |

### Error cases

| TC | Pri | Name | Description |
|----|-----|------|-------------|
| 17 | P1 | `future_block_errors` | N+1000 returns error |
| 18 | P2 | `non_existent_address_returns_zero` | Random address returns 0 |

## Cross-endpoint invariants

- Sealed block nonce never changes after sealing.
- `pending` == `latest` nonce in this rollup.
- Nonce at block N == nonce at block N-1 + tx_count_from_address_in_block_N.

## Contract deployment

None required. ETH transfers suffice for nonce testing.

## References

- [Ethereum JSON-RPC: eth_getTransactionCount](https://ethereum.org/developers/docs/apis/json-rpc/#eth_gettransactioncount)
- [EIP-1898: Typed transaction envelope](https://eips.ethereum.org/EIPS/eip-1898)
