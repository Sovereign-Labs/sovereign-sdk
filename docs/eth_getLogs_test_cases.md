# eth_getLogs test cases

## Summary
- Returns logs that match a filter, scoped by block range or block hash.
- Logs are ordered by block number, transaction index, then log index.
- This rollup uses soft-confirmation semantics: `latest` == `pending`, and logs are expected to be visible immediately.

## Parameters (filter object)
- `fromBlock` / `toBlock`: block tag or hex number; inclusive range.
  - Defaults: if omitted, assume `latest` (which equals `pending` for this rollup).
  - Valid tags: `earliest`, `latest`, `pending`, `safe`, `finalized`
- `blockHash`: 32-byte hash; mutually exclusive with `fromBlock`/`toBlock`.
- `address`: single address or array of addresses (OR semantics).
- `topics`: array (len <= 4). Each position may be:
  - `null` (wildcard),
  - a single topic,
  - an array of topics (OR semantics for that position).

## Curl examples

### Basic: get all logs in a block range
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "0x1",
      "toBlock": "latest"
    }],
    "id": 1
  }'
```

### Filter by contract address
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "0x0",
      "toBlock": "latest",
      "address": "0x1234567890abcdef1234567890abcdef12345678"
    }],
    "id": 1
  }'
```

### Filter by multiple addresses (OR)
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "earliest",
      "toBlock": "latest",
      "address": [
        "0x1111111111111111111111111111111111111111",
        "0x2222222222222222222222222222222222222222"
      ]
    }],
    "id": 1
  }'
```

### Filter by event signature (topic0)
```bash
# Example: Transfer(address,address,uint256) event signature
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "0x0",
      "toBlock": "latest",
      "topics": [
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
      ]
    }],
    "id": 1
  }'
```

### Filter with null wildcard (any value in position)
```bash
# Match any "from" address, specific "to" address
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "0x0",
      "toBlock": "latest",
      "topics": [
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        null,
        "0x000000000000000000000000a94f5374fce5edbc8e2a8697c15331677e6ebf0b"
      ]
    }],
    "id": 1
  }'
```

### Filter with topic OR (multiple values in one position)
```bash
# Match transfers TO address A OR address B
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "0x0",
      "toBlock": "latest",
      "topics": [
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        null,
        [
          "0x000000000000000000000000aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "0x000000000000000000000000bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ]
      ]
    }],
    "id": 1
  }'
```

### Query by block hash (instead of range)
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "blockHash": "0x7c5a35e9cb3e8ae0e221ab470abae9d446c3a5626ce6689fc777dcffcab52c70"
    }],
    "id": 1
  }'
```

### Query pending/latest block
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getLogs",
    "params": [{
      "fromBlock": "pending",
      "toBlock": "pending"
    }],
    "id": 1
  }'
```

### Example response
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": [
    {
      "address": "0x1234567890abcdef1234567890abcdef12345678",
      "topics": [
        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
        "0x0000000000000000000000000000000000000000000000000000000000000001",
        "0x0000000000000000000000000000000000000000000000000000000000000002"
      ],
      "data": "0x0000000000000000000000000000000000000000000000000000000000000064",
      "blockNumber": "0x5",
      "transactionHash": "0xabc123...",
      "transactionIndex": "0x0",
      "blockHash": "0xdef456...",
      "logIndex": "0x0",
      "removed": false
    }
  ]
}
```

## Real-world usage patterns

How modern Ethereum tools use `eth_getLogs`:

### ethers.js / viem
- `provider.getLogs(filter)` wraps the RPC call
- Libraries batch requests, which can cause "too many eth_getLogs in batch" errors
- Wallet providers (MetaMask, WalletConnect) may not support `eth_getLogs` as it's a "node" RPC method
- viem uses tree-shakable `getLogs(publicClient, filter)` actions

### Block explorers & indexers
- Query by address to show all contract events
- Query by topic0 (event signature) to find specific event types
- Use `blockHash` filter for single-block queries after receiving block via subscription
- Paginate large ranges using binary search on block numbers

### dApp frontends
- Subscribe to `newHeads`, then call `eth_getLogs` with `blockHash` for new block's logs
- Filter by contract address + event signature for specific protocol events
- Use topic OR filters for monitoring multiple token transfers

### Common patterns
```
# Event indexing: get all Transfer events for a token
topics: [keccak256("Transfer(address,address,uint256)")]
address: <token_contract>

# Wallet history: get all events involving an address
topics: [null, <padded_address>]  # as sender (topic1)
topics: [null, null, <padded_address>]  # as receiver (topic2)

# Multi-contract monitoring: OR multiple addresses
address: [<contract1>, <contract2>, <contract3>]
```

## Known client behavior differences

Behavior varies across Ethereum clients (Geth, Erigon, Nethermind, Reth):

| Behavior | Geth | Erigon | Notes |
|----------|------|--------|-------|
| `pending` tag | Returns empty array | Returns empty array | Neither fully supports pending logs via `eth_getLogs` |
| `finalized` tag | Supported | Was broken (issue #8199) | May need testing |
| `safe` tag | Supported | May vary | Recent addition |
| Logs timing | Immediate | May lag after `newHeads` | Race condition possible |
| Error on large range | Returns error | Returns empty or error | Inconsistent |

**Implications for this rollup:**
- Our `pending` == `latest` behavior is non-standard but useful for soft-confirmation UX
- Tools expecting Geth/Erigon behavior may be surprised by immediate pending log visibility
- Block explorers typically don't query `pending`, so this is mainly relevant for dApp UX

## Provider limits (for reference)

| Provider | Block range limit | Response limit |
|----------|-------------------|----------------|
| Alchemy | 2K blocks (unlimited logs) OR any range (10K logs) | 150MB |
| Infura | Varies | 10K logs |
| QuickNode | Varies by plan | Varies |
| This rollup | Configurable | Configurable via `eth_getLogsWithCursor` |

## Rollup-specific semantics (assumptions)
- `latest` and `pending` return the same soft-confirmed view.
- Logs from the soft-confirmed block include a synthetic `blockHash`, and that hash is queryable via `blockHash`.
- `safe` and `finalized` map to the last sealed (canonical) head and should exclude pending logs (confirm if needed).
- `removed` is always `false` in `eth_getLogs` responses.
- Error-shape and invalid-param tests are out of scope for this PR.

## Response schema (per Ethereum JSON-RPC spec)

Each log object in the response array must contain:

| Field | Type | Description |
|-------|------|-------------|
| `logIndex` | `QUANTITY` (hex) | Log index position in the block |
| `transactionIndex` | `QUANTITY` (hex) | Transaction index position in the block |
| `transactionHash` | `DATA` (32 bytes) | Hash of the transaction that emitted this log |
| `blockHash` | `DATA` (32 bytes) | Hash of the block containing this log |
| `blockNumber` | `QUANTITY` (hex) | Block number containing this log |
| `address` | `DATA` (20 bytes) | Address of the contract that emitted this log |
| `data` | `DATA` | Non-indexed arguments of the log |
| `topics` | `Array<DATA>` | Array of 0-4 indexed log arguments (32 bytes each) |
| `removed` | `Boolean` | `true` if log was removed due to chain reorg (always `false` for this rollup) |

## Log shapes covered in tests

These cases intentionally vary event signatures, indexed topics, and data payloads:
- `SimpleLog` (`emitLogs`, `emitConfigurableLogs`, `set`): 3 indexed topics + 1 data word (topics = signature, sender, topic1, topic2).
- `FullTopicLog` (`emitFullTopicLog`): 3 indexed topics + 1 data word, max topic length with a distinct signature.
- `DataOnlyLog` (`emitDataOnlyLog`): 0 indexed topics (topics length 1 for signature), data has two uint256 words.

Transaction sources covered:
- Single-log transactions (`emitFullTopicLog`, `emitDataOnlyLog`).
- Multi-log transactions (`emitLogs`, `emitConfigurableLogs`).
- Mixed event signatures in the same block (SimpleLog + DataOnlyLog).

## Test cases

Priority legend: P0 = must-have correctness, P1 = high value, P2 = medium value, P3 = optional.

Block selector semantics:
- TC01 [P0]: `latest` and `pending` ranges return identical logs (paused sequencer, same filter).
- TC02 [P0]: pending logs include a synthetic `blockHash` and non-zero `blockTimestamp`.
- TC03 [P0]: `blockHash` filter for the synthetic block returns logs for all transactions in the pending block up to and including the user's tx (prefix semantics; expected to fail today).
- TC04 [P2]: `safe`/`finalized` return a subset of `latest` and exclude pending (assumption-dependent).
- TC05 [P1]: inclusive boundaries: `fromBlock == toBlock` returns only that block's logs.
- TC06 [P1]: `fromBlock` omitted defaults to `latest`, `toBlock` omitted defaults to `latest`.
- TC07 [P2]: `fromBlock > toBlock` returns an empty result (no error-shape assertions).

Filter semantics:
- TC08 [P0]: address filter matches a single contract.
- TC09 [P1]: address filter with multiple addresses returns the OR of both contracts.
- TC10 [P0]: topic OR semantics: `topics[1] = [A, B]` returns logs with topic A or B.
- TC11 [P1]: topic AND with wildcards: `topics[0] = sig, topics[1] = A, topics[2] = null, topics[3] = B`.
- TC12 [P1]: address + topics intersection (both address and topics must match).
- TC13 [P0]: topic filtering works for pending (`latest`/`pending`) ranges.

Ordering and invariants:
- TC14 [P1]: logs are ordered by block number, transaction index, and log index across a multi-block range.
- TC15 [P1]: `removed` is always `false`.

Related limit behavior (non-error-shape):
- TC16 [P1]: when response size would exceed limits, `eth_getLogsWithCursor` can retrieve the full set for the same filter; `eth_getLogs` may return an error without asserting shape.

Optional extension (non-standard field):
- TC17 [P3]: `timeExecutedMs` is present (via raw JSON-RPC), and logs within the same transaction share the same value.

Additional coverage:
- TC18 [P1]: `earliest` tag returns logs starting from block 0.
- TC19 [P1]: schema validation - all required fields present with correct types (hex quantities, 32-byte hashes, 20-byte addresses).
- TC20 [P1]: EIP-1898 `blockHash` object syntax (`{"blockHash": "0x...", "requireCanonical": true}`) works for `fromBlock`/`toBlock`.
- TC21 [P2]: empty result - filter matching no logs returns empty array `[]`, not an error.
- TC22 [P2]: `FullTopicLog` uses max topics (4 including signature) and data matches the argument.

Log payload coverage (event shapes):
- TC36 [P1]: `DataOnlyLog` has topics length 1 and data decodes to the two uint256 values.
- TC37 [P1]: `topic0` filter selects only the matching event signature when multiple event types are present.
- TC38 [P1]: `SimpleLog` data field matches non-indexed values across multiple logs (non-zero data).

Topic filter edge cases (based on Ethereum spec):
- TC23 [P1]: empty topics array `[]` matches all logs (wildcard).
- TC24 [P1]: `topics: [A]` matches logs with A in first position and anything after.
- TC25 [P1]: `topics: [null, B]` matches any first topic AND B in second position.
- TC26 [P2]: `topics: [[A, B]]` matches A OR B in first position (array at position 0).
- TC27 [P2]: `topics: [[A, B], [C, D]]` matches (A OR B) in pos0 AND (C OR D) in pos1.
- TC28 [P2]: trailing nulls in topics array are effectively ignored (same as shorter array).

Block hash + filter combinations:
- TC29 [P1]: `blockHash` + `address` filter returns only matching address logs from that block.
- TC30 [P1]: `blockHash` + `topics` filter returns only matching topic logs from that block.

Ordering edge cases:
- TC31 [P2]: multiple transactions in same block - logs interleaved correctly by tx index then log index.
- TC32 [P2]: single transaction emitting multiple logs - log indices are sequential starting from 0.

Data field correctness:
- TC33 [P2]: `data` field contains correctly ABI-encoded non-indexed event parameters.
- TC34 [P2]: event with no non-indexed parameters has `data: "0x"`.

Block number format:
- TC35 [P3]: block numbers in response are hex-encoded (e.g., `"0x5"` not `5` or `"5"`).

## Test case to implementation mapping

| Test Case | Implemented Test Function | Status |
|-----------|---------------------------|--------|
| TC01 | `get_logs_latest_and_pending_match` | ✅ |
| TC02 | `get_log_from_pending_block` | ✅ |
| TC03 | `get_log_from_pending_block` (partial) | ⚠️ |
| TC04 | `get_logs_safe_finalized_exclude_pending` | ✅ |
| TC05 | `get_logs_single_block_range` | ✅ |
| TC06 | `get_logs_default_range_matches_latest` | ✅ |
| TC07 | `get_logs_from_greater_than_to_is_empty` | ✅ |
| TC08 | `get_logs_address_filter_single` | ✅ |
| TC09 | `get_logs_address_filter_multiple` | ✅ |
| TC10 | `get_logs_topic_or_semantics` | ✅ |
| TC11 | `get_logs_topic_and_with_wildcard` | ✅ |
| TC12 | `get_logs_address_and_topic_intersection` | ✅ |
| TC13 | `get_logs_pending_with_topic_filter` | ✅ |
| TC14 | `get_logs_ordered_and_not_removed` | ✅ |
| TC15 | `get_logs_ordered_and_not_removed` | ✅ |
| TC16 | `evm_test_get_logs_with_cursor` | ✅ |
| TC17 | `get_logs_time_executed_ms_per_tx` | ✅ |
| TC18 | `get_logs_earliest_tag` | ✅ |
| TC19 | `get_logs_schema_correctness` | ✅ |
| TC20 | `get_logs_eip1898_blockhash_object` | ✅ |
| TC21 | `get_logs_empty_result` | ✅ |
| TC22 | `get_logs_full_topic_log` | ✅ |
| TC23 | `get_logs_empty_topics_matches_all` | ✅ |
| TC24 | `get_logs_topic0_only_matches_any_indexed` | ✅ |
| TC25 | `get_logs_topic1_without_topic0` | ✅ |
| TC26 | `get_logs_topic0_or_semantics_multiple` | ✅ |
| TC27 | `get_logs_topic0_and_topic1_or_semantics` | ✅ |
| TC28 | `get_logs_trailing_null_topics_ignored` | ✅ |
| TC29 | `get_logs_full_topic_log` | ✅ |
| TC30 | `evm_test_get_logs` | ✅ |
| TC31 | `evm_test_get_logs` | ✅ |
| TC32 | `get_logs_emitted_fields_match_event` | ✅ |
| TC33 | `get_logs_emitted_fields_match_event` | ✅ |
| TC34 | - | 🔲 TODO (requires new event with no non-indexed params) |
| TC35 | `get_logs_schema_correctness` | ✅ |
| TC36 | `get_logs_data_only_log` | ✅ |
| TC37 | `get_logs_topic0_filters_event_signature` | ✅ |
| TC38 | `get_logs_emitted_fields_match_event` | ✅ |

## References

- [Ethereum JSON-RPC Specification](https://ethereum.org/developers/docs/apis/json-rpc/)
- [Alchemy eth_getLogs Deep Dive](https://www.alchemy.com/docs/deep-dive-into-eth_getlogs)
- [MetaMask eth_getLogs Documentation](https://docs.metamask.io/services/reference/ethereum/json-rpc-methods/eth_getlogs/)
- [Chainstack: Understanding eth_getLogs Limitations](https://docs.chainstack.com/docs/understanding-eth-getlogs-limitations)
- [ethers.js Provider Documentation](https://docs.ethers.org/v5/api/providers/provider/)
- [viem FAQ](https://viem.sh/docs/faq)
