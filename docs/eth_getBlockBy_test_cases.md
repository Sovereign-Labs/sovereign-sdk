# eth_getBlockByNumber / eth_getBlockByHash test cases

## Summary
- Returns a block object containing header and transactions.
- `eth_getBlockByNumber` accepts block tags or numeric block numbers.
- `eth_getBlockByHash` accepts a 32-byte block hash.
- Both methods have a `details` parameter: `true` returns full transaction objects, `false` returns only tx hashes.
- Critical for block explorers, wallets checking confirmations, and dApps querying historical state.

## Parameters

### eth_getBlockByNumber
- `blockId`: Block number (hex) or tag (`earliest`, `latest`, `pending`, `safe`, `finalized`).
  - Also accepts EIP-1898 object: `{"blockHash": "0x...", "requireCanonical": bool}`.
- `details`: Boolean (default `false`).
  - `false`: transactions array contains 32-byte hashes.
  - `true`: transactions array contains full transaction objects.

### eth_getBlockByHash
- `blockHash`: 32-byte block hash.
- `details`: Boolean (same as above).

## Curl examples

### Get block by number (hashes only)
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getBlockByNumber",
    "params": ["latest", false],
    "id": 1
  }'
```

### Get block by number (full transactions)
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getBlockByNumber",
    "params": ["0x5", true],
    "id": 1
  }'
```

### Get block by hash
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getBlockByHash",
    "params": ["0x7c5a35e9cb3e8ae0e221ab470abae9d446c3a5626ce6689fc777dcffcab52c70", true],
    "id": 1
  }'
```

### EIP-1898 blockHash object
```bash
curl -X POST http://localhost:8545 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "eth_getBlockByNumber",
    "params": [{"blockHash": "0x7c5a35e9cb3e8ae0e221ab470abae9d446c3a5626ce6689fc777dcffcab52c70"}, false],
    "id": 1
  }'
```

## Response schema

### Block object
| Field | Type | Sealed block | Pending block | Notes |
|-------|------|--------------|---------------|-------|
| `number` | QUANTITY (hex) | Block number | sealed_head + 1 | |
| `hash` | DATA (32 bytes) | Block hash | Synthetic hash (non-zero); L1 returns `null` | **Divergence** |
| `parentHash` | DATA (32 bytes) | Parent block hash | Sealed head's hash | |
| `nonce` | DATA (8 bytes) | `0x0000000000000000` | `0x0000000000000000` | PoS blocks |
| `sha3Uncles` | DATA (32 bytes) | EMPTY_OMMER_ROOT_HASH | EMPTY_OMMER_ROOT_HASH | Always empty |
| `logsBloom` | DATA (256 bytes) | Bloom filter | Derived from pending receipts (zero if no logs) | |
| `transactionsRoot` | DATA (32 bytes) | Merkle root | Computed from pending txs | |
| `stateRoot` | DATA (32 bytes) | State root | Synthetic (not actual state root) | |
| `receiptsRoot` | DATA (32 bytes) | Receipts root | Computed from pending receipts | |
| `miner` | DATA (20 bytes) | Beneficiary address | Zero address | |
| `difficulty` | QUANTITY | 0 | 0 | PoS |
| `totalDifficulty` | QUANTITY | 0 | 0 | PoS |
| `extraData` | DATA | May vary | Empty | |
| `size` | QUANTITY | RLP-encoded size | RLP-encoded size for synthetic block | |
| `gasLimit` | QUANTITY | Block gas limit | Block gas limit | |
| `gasUsed` | QUANTITY | Actual gas used | Sum of pending receipt gasUsed | |
| `timestamp` | QUANTITY | Block timestamp | Current timestamp | |
| `transactions` | Array | Hashes or full txs | Hashes or full txs | Based on `details` |
| `uncles` | Array | `[]` | `[]` | Always empty |
| `baseFeePerGas` | QUANTITY | Base fee | Base fee | EIP-1559 |
| `withdrawals` | Array | `null` | `null` | Not supported |
| `withdrawalsRoot` | DATA | `null` | `null` | Not supported |

### Transaction object (when `details=true`)
| Field | Type | Notes |
|-------|------|-------|
| `hash` | DATA (32 bytes) | Transaction hash |
| `nonce` | QUANTITY | Sender's nonce |
| `blockHash` | DATA (32 bytes) | Containing block hash (synthetic for pending) |
| `blockNumber` | QUANTITY | Containing block number |
| `transactionIndex` | QUANTITY | Index within block |
| `from` | DATA (20 bytes) | Sender address |
| `to` | DATA (20 bytes) | Recipient or `null` for contract creation |
| `value` | QUANTITY | Wei transferred |
| `gas` | QUANTITY | Gas limit |
| `gasPrice` | QUANTITY | Gas price (legacy) |
| `input` | DATA | Call data |
| `v`, `r`, `s` | QUANTITY | Signature components |
| `type` | QUANTITY | Transaction type (0, 1, or 2) |
| `maxFeePerGas` | QUANTITY | EIP-1559 max fee |
| `maxPriorityFeePerGas` | QUANTITY | EIP-1559 priority fee |
| `accessList` | Array | EIP-2930 access list |
| `chainId` | QUANTITY | Chain ID |

## Implementation notes and deviations

### Rollup-specific behavior (intended)
- `latest` resolves to `pending` (both return the pending block being constructed).
- `safe` and `finalized` both resolve to the last sealed block (same as `eth_blockNumber`).
- `requireCanonical` in EIP-1898 is accepted but ignored (no reorgs in this rollup).
- If there are no pending txs, `pending`/`latest` fall back to the latest sealed block.
- Pending blocks use a **synthetic hash** (non-zero) and are resolvable via `eth_getBlockByHash`.
- Pending block fields (logsBloom, roots, size, gasUsed) are computed from pending txs/receipts.

### Known deviations vs Ethereum L1 (bugs to track)
1. **`latest` == `pending`**: Both return the pending block. L1: `latest` = last sealed, `pending` = being constructed.
2. **`safe`/`finalized` == head**: Both map to latest sealed block. L1: these may lag behind `latest` based on finality.
3. **Pending hash is synthetic (non-null)**: L1 returns `null` for pending block hash.

## Real-world usage patterns

### Block explorers
- Query blocks sequentially to build chain view
- Use `eth_getBlockByHash` after receiving hash from logs/receipts
- Need accurate `size`, `gasUsed`, `timestamp` for display
- Often query with `details=true` to show transactions

### Wallets (confirmation tracking)
- Get current block via `eth_getBlockByNumber("latest")`
- Compare with tx's `blockNumber` to calculate confirmations
- **Critical**: `latest` must return sealed block for accurate confirmation count

### dApp state queries
- Query block at specific height for historical state context
- Use `eth_getBlockByNumber(N)` before calling `eth_call` at that block
- Rely on `timestamp` for time-based logic

### Indexers & analytics
- Build complete block database by iterating `eth_getBlockByNumber(0, 1, 2, ...)`
- Cross-reference with `eth_getBlockReceipts` for full picture
- Expect `parentHash` chain integrity

## Test cases

Priority legend: P0 = must-have correctness, P1 = high value, P2 = medium value, P3 = optional.

---

### Block tag semantics

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC01 | P0 | `earliest` returns genesis block | `number == 0`, `parentHash == 0x0...0` or genesis-defined |
| TC02 | P0 | Numeric `0` returns same as `earliest` | All fields identical to TC01 |
| TC03 | P0 | `latest` returns pending block | **Divergence**: `number == sealed_head + 1`, `hash != 0x0` (synthetic) |
| TC04 | P0 | `pending` returns pending block | Same as TC03 |
| TC05 | P0 | `safe` returns sealed head | `number == eth_blockNumber()`, `hash` is non-zero |
| TC06 | P0 | `finalized` returns sealed head | Same as TC05 |
| TC07 | P1 | `latest` and `pending` return identical blocks | All fields equal (document divergence from L1) |
| TC08 | P1 | `safe` and `finalized` return identical blocks | All fields equal |
| TC09 | P1 | Numeric block returns exact match | `get_block(N).number == N` |
| TC10 | P1 | Non-existent block (future) returns `null` | `get_block(9999999) == None` |
| TC11 | P2 | Numeric block 1 (first after genesis) | `number == 1`, `parentHash == genesis.hash` |

---

### Sealed vs pending block differences

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC12 | P0 | Sealed block has real hash | `hash != 0x0`, `hash` is 32 bytes |
| TC13 | P0 | Pending block has synthetic hash | `hash != 0x0` (divergence: L1 returns `null`) |
| TC14 | P0 | Pending block number == sealed + 1 | `pending.number == sealed_head.number + 1` |
| TC15 | P0 | Pending parentHash == sealed hash | `pending.parentHash == sealed_head.hash` |
| TC16 | P1 | Sealed block has non-zero size | `size > 0` (actual RLP-encoded size) |
| TC17 | P1 | Pending block has non-zero size | `size > 0` when pending txs exist |
| TC18 | P1 | Sealed block gasUsed reflects actual | `gasUsed == sum(tx.gasUsed)` or 0 if empty |
| TC19 | P1 | Pending block gasUsed reflects pending txs | `gasUsed > 0` when pending txs exist |
| TC20 | P2 | Sealed block has real logsBloom | Derived from transaction logs |
| TC21 | P2 | Pending block logsBloom reflects pending logs | Zero if no logs |

---

### Transaction serialization (`details` parameter)

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC22 | P0 | `details=false`: transactions are hashes | Array of 32-byte hex strings |
| TC23 | P0 | `details=true`: transactions are objects | Array of full tx objects |
| TC24 | P0 | Transaction count matches in both modes | `len(hashes) == len(objects)` |
| TC25 | P1 | Full tx objects have all required fields | See schema above |
| TC26 | P1 | Tx `blockHash` matches containing block | `tx.blockHash == block.hash` |
| TC27 | P1 | Tx `blockNumber` matches containing block | `tx.blockNumber == block.number` |
| TC28 | P1 | Tx `transactionIndex` is sequential | 0, 1, 2, ... within block |
| TC29 | P2 | Tx `hash` in hashes mode matches full tx | `hashes[i] == full_txs[i].hash` |
| TC30 | P2 | Empty block: both modes return empty array | `transactions == []` |
| TC31 | P2 | Contract creation: `to == null` in tx object | `tx.to == null` for deployment |

---

### eth_getBlockByHash

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC32 | P0 | Valid sealed hash returns block | Block matches `get_block_by_number(N)` |
| TC33 | P0 | Non-existent hash returns `null` | Random hash -> `None` |
| TC34 | P0 | Zero hash returns `null` | `get_block_by_hash(0x0...) == None` |
| TC35 | P1 | `details` param works same as by_number | Hashes vs full txs |
| TC36 | P1 | Round-trip: get block, extract hash, get by hash | All fields identical |

---

### Parent hash chain integrity

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC37 | P0 | `block[N].parentHash == block[N-1].hash` | For N > 0, always holds |
| TC38 | P1 | Genesis parentHash is zero or predefined | `block[0].parentHash == 0x0...0` |
| TC39 | P1 | Chain integrity over 5+ blocks | All parent hashes link correctly |

---

### Cross-endpoint consistency

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC40 | P0 | `eth_blockNumber == safe.number` | `eth_blockNumber() == get_block("safe").number` |
| TC41 | P0 | `eth_blockNumber == finalized.number` | `eth_blockNumber() == get_block("finalized").number` |
| TC42 | P1 | Block by number == block by hash | `get_block(N) == get_block(get_block(N).hash)` |
| TC43 | P1 | `pending.number` vs `eth_blockNumber` | Pending is one ahead when pending txs exist; otherwise equals sealed |
| TC44 | P2 | Block timestamp <= current time | `block.timestamp <= now()` |
| TC45 | P2 | Block timestamps increase monotonically | `block[N].timestamp >= block[N-1].timestamp` |

---

### EIP-1898 blockHash parameter

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC46 | P1 | `{"blockHash": H}` returns same as by_hash | All fields identical |
| TC47 | P2 | `{"blockHash": H, "requireCanonical": true}` | Same as TC46 (no reorgs) |
| TC48 | P2 | `{"blockHash": H, "requireCanonical": false}` | Same as TC46 |
| TC49 | P2 | Non-existent hash in EIP-1898 object | Returns `null` |

---

### State-dependent scenarios

#### Empty blocks (no transactions)

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC50 | P0 | Empty block has `transactions == []` | Both `details` modes |
| TC51 | P1 | Empty block has `gasUsed == 0` | |
| TC52 | P2 | Multiple consecutive empty blocks | All have `transactions == []`, `gasUsed == 0` |

#### Block with single transaction

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC53 | P0 | Block with 1 tx: count correct | `len(transactions) == 1` |
| TC54 | P0 | Tx hash in block matches receipt | `block.txs[0] == receipt.transactionHash` |
| TC55 | P1 | Block gasUsed matches receipt gasUsed | `block.gasUsed == receipt.gasUsed` |
| TC56 | P1 | Tx `from` matches sender address | Known sender address |
| TC57 | P1 | Tx `to` matches recipient | For transfers |
| TC58 | P1 | Tx `value` matches sent amount | For transfers |

#### Block with multiple transactions

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC59 | P0 | Block with N txs: count correct | `len(transactions) == N` |
| TC60 | P0 | Transaction indices are sequential | 0, 1, 2, ..., N-1 |
| TC61 | P1 | Block gasUsed == sum of all tx gasUsed | Cross-check with receipts |
| TC62 | P1 | Transaction order matches submission order | Deterministic ordering |
| TC63 | P2 | Different tx types in same block | Legacy, EIP-1559 |

#### Contract deployment

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC64 | P0 | Deployment tx has `to == null` | |
| TC65 | P1 | Block with deployment has non-zero gasUsed | Contract creation costs gas |
| TC66 | P2 | Multiple deployments in one block | Each tx has `to == null` |

#### Mixed transaction types

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC67 | P1 | Block with transfer + contract call | Both tx types correct |
| TC68 | P2 | Block with deployment + call to deployed | Correct `to` values |

---

### Value correctness cross-checks

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC69 | P0 | `baseFeePerGas` matches `eth_feeHistory` | For same block number |
| TC70 | P0 | Receipts `blockHash` matches block hash | `receipt.blockHash == block.hash` |
| TC71 | P1 | Receipts `blockNumber` matches block number | `receipt.blockNumber == block.number` |
| TC72 | P1 | `gasLimit` is positive and reasonable | `gasLimit > 0`, typically 30M+ |
| TC73 | P2 | `timestamp` is reasonable Unix timestamp | > 1600000000 (post-2020) |

---

### Block progression scenarios

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC74 | P0 | After producing block, sealed head advances | `eth_blockNumber` increases |
| TC75 | P0 | New sealed block has previous pending's txs | Transactions move from pending to sealed |
| TC76 | P1 | `eth_blockNumber` never decreases | Monotonic within session |
| TC77 | P1 | Sealed block data is immutable | Query twice, get identical results |
| TC78 | P2 | Query during block production | Consistent view (pause before assertion) |

---

### Header field correctness

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC79 | P0 | `sha3Uncles` is empty ommer hash | `0x1dcc4de8dec75d7aab85b567b6ccd41ad312451b948a7413f0a142fd40d49347` |
| TC80 | P1 | `nonce` is zero (PoS) | `0x0000000000000000` |
| TC81 | P1 | `difficulty` is 0 (PoS) | |
| TC82 | P2 | `mixHash` is present | May be zero or hash |
| TC83 | P2 | `uncles` array is empty | `[]` |
| TC84 | P2 | `withdrawals` is null | Not supported |

---

### Extended state scenarios

#### Long-running chain

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC85 | P1 | Block at height 10+ | All fields valid, parent chain intact |
| TC86 | P1 | Query historical block after many new blocks | Data unchanged from when first sealed |
| TC87 | P2 | First block after genesis vs block 10+ | Same schema, different values |

#### High gas usage blocks

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC88 | P1 | Block near gas limit | `gasUsed / gasLimit` ratio close to 1.0 |
| TC89 | P1 | Block with expensive contract deployment | `gasUsed > 100000` |
| TC90 | P2 | Block with many small transactions | `gasUsed == sum(individual gasUsed)` |

#### Failed/reverted transactions

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC91 | P0 | Block containing reverted tx | Tx still in `transactions` array |
| TC92 | P1 | Reverted tx gasUsed counted in block | `block.gasUsed` includes reverted tx gas |
| TC93 | P2 | Reverted tx fields still correct | `from`, `to`, `value`, `input` unchanged |

#### Pending block with waiting transactions

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC94 | P0 | Pending block shows submitted txs | `pending.transactions` includes waiting txs |
| TC95 | P1 | Pending tx count matches mempool | After N submissions, `len(pending.txs) == N` |
| TC96 | P1 | Pending tx has correct fields | `from`, `value`, `input` match submitted |
| TC97 | P2 | Pending tx `blockHash` handling | `tx.blockHash` matches pending block hash (synthetic, non-null) |

#### Multiple transactions from same sender

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC98 | P1 | Two txs from same sender in block | Both in `transactions`, correct order |
| TC99 | P1 | Sender nonces in tx objects | Nonces are sequential (N, N+1) |
| TC100 | P2 | Three txs from same sender | All three present, nonces N, N+1, N+2 |

#### Blocks with events/logs

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC101 | P1 | Block with event-emitting tx | `logsBloom` is non-zero |
| TC102 | P1 | Block with multiple events | `logsBloom` covers all event topics |
| TC103 | P2 | Empty block vs block with events | Empty has zero bloom, events has non-zero |

#### Transaction type variations

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC104 | P1 | Block with EIP-1559 tx (type 2) | `tx.type == 2`, has `maxFeePerGas` |
| TC105 | P2 | Block with legacy tx (type 0) | `tx.type == 0`, has `gasPrice` |
| TC106 | P2 | Block with access list tx (type 1) | `tx.type == 1`, has `accessList` |
| TC107 | P2 | Mixed tx types in one block | Each tx has correct type-specific fields |

---

### Comprehensive value cross-checks

#### Cross-check with eth_getBlockReceipts

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC108 | P0 | Receipt count matches tx count | `len(receipts) == len(block.transactions)` |
| TC109 | P0 | All receipts have same blockHash | `receipt[i].blockHash == block.hash` |
| TC110 | P0 | All receipts have same blockNumber | `receipt[i].blockNumber == block.number` |
| TC111 | P1 | Receipt order matches tx order | `receipt[i].transactionIndex == i` |
| TC112 | P1 | Block gasUsed == sum of receipt gasUsed | Exact match |

#### Cross-check with eth_getTransactionByHash

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC113 | P0 | Tx from block matches tx by hash | `get_tx(hash) == block.txs[i]` for full tx mode |
| TC114 | P1 | Tx blockHash matches containing block | `tx.blockHash == block.hash` |
| TC115 | P1 | Tx blockNumber matches containing block | `tx.blockNumber == block.number` |
| TC116 | P1 | Tx transactionIndex is correct | `tx.transactionIndex == index in block` |

#### Cross-check with eth_getTransactionReceipt

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC117 | P0 | Receipt blockHash matches block | `receipt.blockHash == block.hash` |
| TC118 | P0 | Receipt blockNumber matches block | `receipt.blockNumber == block.number` |
| TC119 | P1 | Receipt txHash matches tx in block | `receipt.transactionHash == block.txs[i]` |
| TC120 | P1 | Receipt status is 1 for successful tx | |
| TC121 | P2 | Receipt cumulativeGasUsed is cumulative | Increases through block |

#### Cross-check logsBloom with logs

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC122 | P1 | Log address is in logsBloom | Bloom filter contains address bits |
| TC123 | P1 | Log topics are in logsBloom | Bloom filter contains topic bits |
| TC124 | P2 | Empty logs -> zero logsBloom | All bloom bytes are 0 |

#### Cross-check with eth_getLogs

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC125 | P0 | Logs for block match logsBloom presence | If logs exist, bloom non-zero |
| TC126 | P1 | All logs have correct blockHash | `log.blockHash == block.hash` |
| TC127 | P1 | All logs have correct blockNumber | `log.blockNumber == block.number` |
| TC128 | P2 | Log transactionHash is in block txs | Hash exists in `block.transactions` |

#### Cross-check with eth_feeHistory

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC129 | P0 | `baseFeePerGas` matches feeHistory | `block.baseFeePerGas == feeHistory.baseFeePerGas[index]` |
| TC130 | P1 | gasUsedRatio derivable from block | `feeHistory.gasUsedRatio[i] == block.gasUsed / block.gasLimit` |

#### Cross-check with eth_blockNumber

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC131 | P0 | eth_blockNumber == get_block("safe").number | Always equal |
| TC132 | P0 | eth_blockNumber == get_block("finalized").number | Always equal |
| TC133 | P1 | get_block("pending").number vs eth_blockNumber | Pending is one ahead when pending txs exist |

#### Transaction field value validation

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC134 | P0 | `tx.from` is valid address | 20 bytes, matches sender |
| TC135 | P0 | `tx.hash` is valid hash | 32 bytes, matches computed hash |
| TC136 | P1 | `tx.value` matches sent amount | Exact Wei value |
| TC137 | P1 | `tx.input` matches calldata | Exact bytes |
| TC138 | P1 | `tx.gas` >= receipt gasUsed | Gas limit >= actual used |
| TC139 | P2 | `tx.nonce` is correct for sender | Matches sender's nonce at that point |
| TC140 | P2 | `tx.chainId` matches chain | Equals `eth_chainId()` |

#### Timestamp consistency

| ID | Priority | Description | Expected values |
|----|----------|-------------|-----------------|
| TC141 | P1 | Block timestamp increases | `block[N].timestamp >= block[N-1].timestamp` |
| TC142 | P1 | Timestamp matches log blockTimestamp | `block.timestamp == log.blockTimestamp` |
| TC143 | P2 | Timestamp is recent Unix time | Within reasonable range of current time |

---

## Test setup and fixtures

### Minimal fixture for most tests
```
State: Produce 3 sealed blocks
- Block 0: Genesis (no txs)
- Block 1: Empty
- Block 2: 1 transfer tx (sender -> recipient, 0.1 ETH)
Pause sequencer before assertions
```

### Multi-transaction fixture
```
State:
- Deploy SimpleStorage contract (block N)
- Call set_value(12345) (block N+1)
- Call set_value(67890) (block N+1, same block if batched)
Record block numbers for each tx
Pause sequencer before assertions
```

### Block hash fixture
```
Record these for cross-checks:
- H0 = genesis hash
- H1 = block 1 hash
- H2 = block 2 hash
- H_pending = pending hash (synthetic, non-zero)
```

## Value derivation rules

### gasUsed
- Empty block: `gasUsed == 0`
- Block with txs: `gasUsed == sum(receipt.gasUsed for all txs)`
- Can cross-check with `eth_getBlockReceipts`

### size
- Sealed block: RLP-encoded size in bytes
- Pending block: RLP-encoded size for synthetic block when pending txs exist
- Genesis block 0 has a specific size (e.g., 507 bytes per existing test)

### timestamp
- Must be Unix timestamp (seconds since epoch)
- Must be >= previous block's timestamp
- Should be close to actual wall clock time

### transactionsRoot, receiptsRoot, stateRoot
- Sealed: Merkle roots computed from block data
- Pending: `transactionsRoot`/`receiptsRoot` computed from pending txs/receipts; `stateRoot` is synthetic

### logsBloom
- Sealed: Bloom filter from all logs in block
- Pending: Bloom filter from pending receipts (all zeros if no logs)

## Test dependencies

- Use `setup_test_rollup(0, EVM_EXTENSION)` for instant finality
- Use `alloy_client(rollup.http_addr)` for Provider access
- Use `pause_preferred_batches()` / `resume_preferred_batches()` for determinism
- For EIP-1898 tests, may need raw JSON-RPC via `jsonrpsee`

---

## Existing Test Coverage Analysis

> Note: This coverage analysis predates the newer tests in `evm_block_by_number_hash.rs` and may be stale. Refresh as needed.

### Files with relevant tests

| File | Tests | Coverage |
|------|-------|----------|
| `evm_rpc.rs` | `eth_get_block_by_number`, `eth_get_block_by_hash`, `block_size` | Basic tags, parent hash |
| `evm_block_number.rs` | `pending_block_number` | Pending header fields |
| `evm_block_hash.rs` | `block_hash` | BLOCKHASH opcode, not RPC |
| `evm_tx.rs` | `sanity_checks`, `execute_evm_tests` | Basic tags, parent hash |
| `evm_soft_conf.rs` | `evm_test_soft_confirmations` | Pending txs, block hash nullity |

### Detailed coverage by test case

#### Block tag semantics (TC01-TC11)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC01 | Partial | `evm_rpc:32`, `evm_tx:35` | Only checks number==0, not parentHash |
| TC02 | Partial | `evm_rpc:32` | No explicit assertion comparing to TC01 |
| TC03 | Covered | `evm_rpc:33`, `evm_tx:40-47` | Documents latest==pending |
| TC04 | Covered | `evm_rpc:34`, `evm_tx:43` | |
| TC05 | Missing | - | `safe` tag not tested |
| TC06 | Missing | - | `finalized` tag not tested |
| TC07 | Covered | `evm_tx:47` | `assert_eq!(latest_block, pending_block)` |
| TC08 | Missing | - | No safe/finalized comparison |
| TC09 | Covered | `evm_rpc:35,46-47` | |
| TC10 | Covered | `evm_rpc:36,48` | Returns None for future block |
| TC11 | Partial | `evm_rpc:50-53` | Parent hash checked but not genesis->1 |

#### Sealed vs pending differences (TC12-TC21)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC12 | Partial | `evm_block_hash:47-48` | Via contract call, not direct assertion |
| TC13 | Covered | `evm_rpc:79`, `evm_tx:48` | Asserts pending hash is non-zero |
| TC14 | Partial | `evm_soft_conf:36,56` | Implicitly via `expected_block_nr + 1` |
| TC15 | Partial | `evm_rpc:73` | Gets parentHash but doesn't assert == sealed.hash |
| TC16 | Covered | `evm_rpc:125-126` | Only for genesis, not sealed with txs |
| TC17 | Missing | - | Pending size not tested |
| TC18 | Missing | - | Sealed gasUsed not tested |
| TC19 | Partial | `evm_block_number:22` | Only checks gasUsed==0 when no pending txs |
| TC20 | Missing | - | logsBloom not tested |
| TC21 | Missing | - | Pending logsBloom not tested |

#### Transaction serialization (TC22-TC31)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC22 | Partial | `evm_soft_conf:57-60` | Uses `.hashes()` but doesn't verify format |
| TC23 | Missing | - | `details=true` never tested |
| TC24 | Missing | - | No comparison of counts in both modes |
| TC25 | Missing | - | Full tx object fields not validated |
| TC26 | Missing | - | tx.blockHash not checked against block.hash |
| TC27 | Missing | - | tx.blockNumber not checked |
| TC28 | Missing | - | transactionIndex not validated |
| TC29 | Missing | - | Hash mode vs full mode hash match not tested |
| TC30 | Partial | `evm_soft_conf:30` | Only checks `is_empty()` |
| TC31 | Missing | - | Contract creation `to == null` not tested |

#### eth_getBlockByHash (TC32-TC36)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC32 | Covered | `evm_rpc:73-76` | Valid hash returns correct block |
| TC33 | Missing | - | Random hash test missing |
| TC34 | Covered | `evm_rpc:81` | Zero hash returns None |
| TC35 | Missing | - | details param not tested for by_hash |
| TC36 | Partial | `evm_rpc:73-76` | Round-trip but doesn't compare all fields |

#### Parent hash chain (TC37-TC39)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC37 | Covered | `evm_rpc:50-53`, `evm_tx:114-117` | |
| TC38 | Missing | - | Genesis parentHash not explicitly tested |
| TC39 | Missing | - | 5+ block chain not tested |

#### Cross-endpoint consistency (TC40-TC45)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC40 | Missing | - | eth_blockNumber vs safe not tested |
| TC41 | Missing | - | eth_blockNumber vs finalized not tested |
| TC42 | Partial | `evm_rpc:73-76` | By number vs by hash tested |
| TC43 | Partial | `evm_soft_conf:36` | Implicit check only |
| TC44 | Missing | - | Timestamp vs current time not tested |
| TC45 | Missing | - | Timestamp monotonicity not tested |

#### EIP-1898 (TC46-TC49)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC46 | Missing | - | blockHash object syntax not tested |
| TC47 | Missing | - | requireCanonical=true not tested |
| TC48 | Missing | - | requireCanonical=false not tested |
| TC49 | Missing | - | Non-existent hash in object not tested |

#### State-dependent scenarios (TC50-TC107)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC50-52 | Partial | `evm_rpc:96-97` | Empty receipts checked, not empty txs |
| TC53-58 | Missing | - | Single tx block validation missing |
| TC59-63 | Missing | - | Multi-tx block validation missing |
| TC64-66 | Missing | - | Contract deployment in block missing |
| TC67-68 | Missing | - | Mixed tx types missing |
| TC85-87 | Missing | - | Long chain scenarios missing |
| TC88-90 | Missing | - | High gas usage missing |
| TC91-93 | Missing | - | Reverted tx in block missing |
| TC94-97 | Partial | `evm_soft_conf:52-62` | Pending txs partially tested |
| TC98-100 | Missing | - | Multiple txs same sender missing |
| TC101-103 | Missing | - | Blocks with events missing |
| TC104-107 | Missing | - | Transaction type variations missing |

#### Value cross-checks (TC108-TC143)

| TC | Status | Covered by | Missing |
|----|--------|------------|---------|
| TC108-112 | Partial | `evm_rpc:96-113` | Receipt count checked, not all fields |
| TC113-116 | Missing | - | tx by hash vs block tx matching missing |
| TC117-121 | Missing | - | Receipt fields vs block missing |
| TC122-124 | Missing | - | logsBloom validation missing |
| TC125-128 | Missing | - | eth_getLogs vs block missing |
| TC129-130 | Partial | `evm_fee_history.rs` | baseFee checked in fee_history tests |
| TC131-133 | Missing | - | eth_blockNumber consistency missing |
| TC134-140 | Missing | - | Transaction field validation missing |
| TC141-143 | Missing | - | Timestamp consistency missing |

### Summary: Coverage gaps

**Critical gaps (P0):**
- `safe` and `finalized` tags (TC05, TC06)
- `details=true` (full transactions) mode (TC23-TC29)
- Cross-check eth_blockNumber vs safe/finalized (TC40-TC41)
- Pending block shows submitted txs correctly (TC94-TC96)
- Receipt fields match block (TC108-TC112 partial, TC117-TC121 missing)

**High-value gaps (P1):**
- EIP-1898 blockHash parameter (TC46-TC49)
- Random non-existent hash test (TC33)
- Long chain scenarios (TC85-TC87)
- Transaction field validation (TC134-TC140)
- Timestamp monotonicity (TC45)
- logsBloom validation (TC122-TC124)

**Medium gaps (P2):**
- Mixed transaction types in block (TC104-TC107)
- Reverted transaction in block (TC91-TC93)
- Genesis parentHash assertion (TC38)

### Validation issues in existing tests

1. **`evm_rpc.rs:by_number` helper only returns Header** - discards transactions, cannot test TC22-TC31
2. **No `details=true` calls anywhere** - full transaction mode untested
3. **No cross-checks between endpoints** - block data not verified against receipts/logs
4. **No safe/finalized tag usage** - these tags completely untested
5. **No value assertions on header fields** - gas_limit, baseFeePerGas, timestamp not validated

## References

- [Ethereum JSON-RPC Specification](https://ethereum.org/developers/docs/apis/json-rpc/)
- [EIP-1898: Typed block parameter](https://eips.ethereum.org/EIPS/eip-1898)
- [QuickNode eth_getBlockByNumber](https://www.quicknode.com/docs/ethereum/eth_getBlockByNumber)
- [Alloy Provider Documentation](https://docs.rs/alloy-provider)
