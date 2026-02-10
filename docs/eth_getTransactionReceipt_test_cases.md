# eth_getTransactionReceipt test cases

## Summary

- Returns the receipt of a transaction by its hash (no block selector/tag parameter).
- L1 behavior: returns `null` for unknown hashes and pending (unmined) transactions.
- Current rollup behavior: pending transactions return receipts with `blockHash` and `blockNumber` set (L1 divergence;
  document via ignored tests, do not assume intentional).
- **This PR must assert real, valid values** derived from fixtures or cross-endpoint checks. Avoid placeholders, zeros,
  or empty values unless the spec explicitly allows them.

## Parameters

- `transactionHash`: 32-byte hex string (required). The hash of the transaction.

## Curl example

```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getTransactionReceipt",
    "params": ["0x85d995eba9763907fdf35cd2034144dd9d53ce32cbec21349d4b12823c6860c5"],
    "id": 1
  }'
```

### Example response (success)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "transactionHash": "0x85d995eba9763907fdf35cd2034144dd9d53ce32cbec21349d4b12823c6860c5",
    "transactionIndex": "0x0",
    "blockHash": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    "blockNumber": "0x5",
    "from": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266",
    "to": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
    "cumulativeGasUsed": "0x5208",
    "gasUsed": "0x5208",
    "contractAddress": null,
    "logs": [],
    "logsBloom": "0x00...",
    "status": "0x1",
    "effectiveGasPrice": "0x3b9aca00",
    "type": "0x2"
  }
}
```

Note: example values are illustrative only; tests should compute and verify actual values.

### Example response (unknown hash)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": null
}
```

## Response schema

| Field               | Type                   | Description                                         |
|---------------------|------------------------|-----------------------------------------------------|
| `transactionHash`   | DATA (32 bytes)        | Hash of the transaction                             |
| `transactionIndex`  | QUANTITY (hex)         | 0-indexed position within block                     |
| `blockHash`         | DATA (32 bytes)        | Hash of containing block                            |
| `blockNumber`       | QUANTITY (hex)         | Number of containing block                          |
| `from`              | DATA (20 bytes)        | Sender address                                      |
| `to`                | DATA (20 bytes) / null | Recipient (null for contract creation)              |
| `contractAddress`   | DATA (20 bytes) / null | Created contract address (null for regular calls)   |
| `cumulativeGasUsed` | QUANTITY (hex)         | Total gas used in block up to and including this tx |
| `gasUsed`           | QUANTITY (hex)         | Gas used by this specific transaction               |
| `effectiveGasPrice` | QUANTITY (hex)         | Actual gas price paid (EIP-1559)                    |
| `status`            | QUANTITY (hex)         | `0x1` for success, `0x0` for failure                |
| `logs`              | Array                  | Array of log objects emitted                        |
| `logsBloom`         | DATA (256 bytes)       | Bloom filter for light clients                      |
| `type`              | QUANTITY (hex)         | Transaction type (0x2 for EIP-1559)                 |
| `root`              | DATA (32 bytes) / null | Pre-Byzantium receipts only (unlikely here)         |
| `blobGasUsed`       | QUANTITY (hex) / null  | EIP-4844 only                                       |
| `blobGasPrice`      | QUANTITY (hex) / null  | EIP-4844 only                                       |

### Log object schema (within `logs` array)

| Field              | Type                  | Description                                                  |
|--------------------|-----------------------|--------------------------------------------------------------|
| `address`          | DATA (20 bytes)       | Contract that emitted the log                                |
| `topics`           | Array<DATA>           | 0-4 indexed log arguments (32 bytes each)                    |
| `data`             | DATA                  | Non-indexed log arguments                                    |
| `blockNumber`      | QUANTITY (hex)        | Block number                                                 |
| `transactionHash`  | DATA (32 bytes)       | Transaction hash                                             |
| `transactionIndex` | QUANTITY (hex)        | Transaction index in block                                   |
| `blockHash`        | DATA (32 bytes)       | Block hash                                                   |
| `logIndex`         | QUANTITY (hex)        | Log index in block                                           |
| `removed`          | Boolean               | `false` for canonical logs (can be `true` after reorgs)      |
| `blockTimestamp`   | QUANTITY (hex) / null | Non-standard; present in this rollup (block timestamp)       |
| `timeExecutedMs`   | QUANTITY (hex) / null | Non-standard; present in this rollup (execution time per tx) |

## Rollup-specific behavior (current implementation)

- Receipts include non-standard log fields `blockTimestamp` and `timeExecutedMs` (per-log).
- Pending txs return receipts (L1 returns null).

**Known L1 divergences (documented, not to fix):**

- **Pending receipts returned**: Unlike L1, receipts ARE returned for pending (unmined) transactions.
- **blockHash set for pending**: `blockHash` is set for pending blocks and may not be retrievable via
  `eth_getBlockByHash` (L1 returns null for receipt).
- **blockNumber set for pending**: `blockNumber` equals pending block number (L1 returns null for receipt).

Reference: `docs/rpc_priority_sorted.md` item #5, `examples/demo-rollup/tests/evm/evm_soft_conf.rs:40-48`,
`examples/demo-rollup/tests/evm/evm_rpc.rs:206`.

## Test cases

Priority legend: P0 = must-have correctness, P1 = high value, P2 = medium value, P3 = optional.

### Basic functionality

| ID   | Priority | Description                                                                   |
|------|----------|-------------------------------------------------------------------------------|
| TC01 | P0       | Mined transaction returns complete receipt with all fields present and valid. |
| TC02 | P0       | Unknown transaction hash returns `null`.                                      |
| TC03 | P1       | Contract deployment: `contractAddress` is set, `to` is null.                  |
| TC04 | P1       | Regular contract call: `contractAddress` is null, `to` is set.                |
| TC05 | P1       | Reverted transaction: `status` is `0x0`.                                      |

### Receipt field correctness

| ID   | Priority | Description                                                                              |
|------|----------|------------------------------------------------------------------------------------------|
| TC06 | P0       | `transactionHash` in response equals the queried hash.                                   |
| TC07 | P0       | `blockHash` matches `eth_getBlockByNumber(blockNumber).hash` for sealed tx.              |
| TC08 | P0       | `blockNumber` is correct for the block containing the transaction.                       |
| TC09 | P1       | `transactionIndex` is 0 for first/only tx in block.                                      |
| TC10 | P1       | `transactionIndex` is sequential (0, 1, 2) for multiple txs in same block.               |
| TC11 | P0       | `from` matches the sender address used to sign the transaction.                          |
| TC12 | P1       | `to` matches the recipient address for regular calls.                                    |
| TC13 | P0       | `gasUsed` is non-zero and <= block gas limit.                                            |
| TC14 | P0       | `cumulativeGasUsed` >= `gasUsed` for same receipt.                                       |
| TC15 | P1       | `cumulativeGasUsed` for second tx in block > first tx's `cumulativeGasUsed`.             |
| TC16 | P1       | `effectiveGasPrice` = `baseFee + min(maxPriorityFee, maxFee - baseFee)` for EIP-1559 tx. |
| TC17 | P0       | `status` is `0x1` for successful transaction.                                            |
| TC18 | P1       | `type` is `0x2` for EIP-1559 transactions.                                               |

### Logs field correctness

| ID   | Priority | Description                                                             |
|------|----------|-------------------------------------------------------------------------|
| TC19 | P0       | Transaction emitting events has non-empty `logs` array.                 |
| TC20 | P1       | Transaction with no events has empty `logs` array.                      |
| TC21 | P1       | Log `address` matches the contract that emitted the event.              |
| TC22 | P1       | Log `topics` contains correct indexed event parameters.                 |
| TC23 | P1       | Log `data` contains correct non-indexed event parameters.               |
| TC24 | P1       | Log `logIndex` values are sequential within the receipt (0, 1, 2, ...). |
| TC25 | P1       | Log `blockHash` equals receipt `blockHash`.                             |
| TC26 | P1       | Log `transactionHash` equals receipt `transactionHash`.                 |
| TC27 | P2       | Log `removed` is always `false`.                                        |

### Pending transaction behavior (L1 divergence documentation)

| ID   | Priority | Description                                                                                                                                |
|------|----------|--------------------------------------------------------------------------------------------------------------------------------------------|
| TC28 | P0       | **L1 parity (ignored)**: Pending tx returns `null`. Pause sequencer, send tx, query receipt. Expected `null` on L1 (currently fails).      |
| TC29 | P1       | **Current behavior (diagnostic)**: Pending tx returns a receipt with `blockHash` and `blockNumber` set.                                    |
| TC30 | P0       | After sealing, receipt is consistent with `eth_getTransactionByHash` and `eth_getBlockByNumber` (block hash/number/index, logs/logsBloom). |

### Cross-endpoint consistency

| ID   | Priority | Description                                                                                          |
|------|----------|------------------------------------------------------------------------------------------------------|
| TC34 | P0       | Receipt matches corresponding entry from `eth_getBlockReceipts(blockNumber)`.                        |
| TC35 | P1       | Receipt `blockHash` matches `eth_getBlockByNumber(blockNumber).hash`.                                |
| TC36 | P1       | Receipt logs are present in `eth_getLogs` filtered by `blockHash` + `address` + `topics`.            |
| TC37 | P1       | Sum of `gasUsed` for all receipts in block equals `block.gasUsed`.                                   |
| TC38 | P2       | `transactionIndex` matches transaction position in `eth_getBlockByNumber(blockNumber).transactions`. |

### Edge cases

| ID   | Priority | Description                                                              |
|------|----------|--------------------------------------------------------------------------|
| TC39 | P2       | Receipt for transaction in block 0 (if any transactions exist).          |
| TC40 | P2       | Transaction emitting 10+ logs has all logs present with correct indices. |
| TC41 | P2       | Transaction to zero address: `to` is `0x0000...0000`, not null.          |
| TC42 | P3       | `logsBloom` is valid 256-byte bloom filter (non-empty for tx with logs). |

## Test case to implementation mapping

| Test Case                                                                          | Implemented Test Function                     | Status                 |
|------------------------------------------------------------------------------------|-----------------------------------------------|------------------------|
| TC02                                                                               | `eth_get_transaction_receipt_unknown_tx`      | ✅                      |
| TC01, TC03, TC04, TC06-08, TC11, TC13-14 (single-tx), TC16-20, TC21-27, TC35, TC42 | `eth_get_transaction_receipt_fields`          | ✅                      |
| TC05                                                                               | `test_allow_publishing_reverted_txs`          | ✅                      |
| TC09, TC10, TC12, TC15, TC34, TC36-38                                              | `eth_get_transaction_receipt_multi_tx_block`  | ✅                      |
| TC28                                                                               | `eth_get_transaction_receipt_pending_is_null` | ⚠️ ignored (L1 parity) |
| Remaining                                                                          | (to be implemented)                           | TODO                   |

## Implementation notes

- Run tests with: `SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup eth_get_transaction_receipt`
- Follow existing test patterns in `evm_soft_conf.rs` and `evm_fee_history.rs`
- Use `pause_preferred_batches()` for deterministic pending block testing
- Use `SimpleStorage` contract for event emission tests
- To assert non-standard log fields (`blockTimestamp`, `timeExecutedMs`), use raw JSON-RPC (typed clients may omit
  them).

## References

- [Ethereum JSON-RPC Specification](https://ethereum.org/developers/docs/apis/json-rpc/)
- [QuickNode eth_getTransactionReceipt](https://www.quicknode.com/docs/ethereum/eth_getTransactionReceipt)
- [MetaMask eth_getTransactionReceipt](https://docs.metamask.io/services/reference/ethereum/json-rpc-methods/eth_gettransactionreceipt/)
