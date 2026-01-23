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
| `hash` | DATA (32 bytes) | Block hash | `null` (L1) / `0x0` (this rollup) | **Divergence** |
| `parentHash` | DATA (32 bytes) | Parent block hash | Sealed head's hash | |
| `nonce` | DATA (8 bytes) | `0x0000000000000000` | `0x0000000000000000` | PoS blocks |
| `sha3Uncles` | DATA (32 bytes) | EMPTY_OMMER_ROOT_HASH | EMPTY_OMMER_ROOT_HASH | Always empty |
| `logsBloom` | DATA (256 bytes) | Bloom filter | Empty bloom | |
| `transactionsRoot` | DATA (32 bytes) | Merkle root | EMPTY_ROOT_HASH | |
| `stateRoot` | DATA (32 bytes) | State root | EMPTY_ROOT_HASH | |
| `receiptsRoot` | DATA (32 bytes) | Receipts root | EMPTY_ROOT_HASH | |
| `miner` | DATA (20 bytes) | Beneficiary address | Zero address | |
| `difficulty` | QUANTITY | 0 | 0 | PoS |
| `totalDifficulty` | QUANTITY | 0 | 0 | PoS |
| `extraData` | DATA | May vary | Empty | |
| `size` | QUANTITY | RLP-encoded size | 0 | |
| `gasLimit` | QUANTITY | Block gas limit | Block gas limit | |
| `gasUsed` | QUANTITY | Actual gas used | 0 | |
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
| `blockHash` | DATA (32 bytes) | Containing block hash (`null` for pending) |
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
- Pending block `hash` is `0x0` (Ethereum L1 returns `null`).
- Pending block `size` is 0 (sealed blocks have actual RLP size).

### Known deviations vs Ethereum L1 (bugs to track)
1. **`latest` == `pending`**: Both return the pending block. L1: `latest` = last sealed, `pending` = being constructed.
2. **`safe`/`finalized` == head**: Both map to latest sealed block. L1: these may lag behind `latest` based on finality.
3. **Pending hash is `0x0`**: L1 returns `null` for pending block hash.

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
| TC03 | P0 | `latest` returns pending block | **Divergence**: `number == sealed_head + 1`, `hash == 0x0` |
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
| TC13 | P0 | Pending block has zero hash | `hash == 0x0` (divergence: L1 returns `null`) |
| TC14 | P0 | Pending block number == sealed + 1 | `pending.number == sealed_head.number + 1` |
| TC15 | P0 | Pending parentHash == sealed hash | `pending.parentHash == sealed_head.hash` |
| TC16 | P1 | Sealed block has non-zero size | `size > 0` (actual RLP-encoded size) |
| TC17 | P1 | Pending block has size 0 | `size == 0` (implementation detail) |
| TC18 | P1 | Sealed block gasUsed reflects actual | `gasUsed == sum(tx.gasUsed)` or 0 if empty |
| TC19 | P1 | Pending block gasUsed is 0 | `gasUsed == 0` |
| TC20 | P2 | Sealed block has real logsBloom | Derived from transaction logs |
| TC21 | P2 | Pending block has empty logsBloom | All zeros |

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
| TC43 | P1 | `pending.number == eth_blockNumber + 1` | Pending is always one ahead of sealed |
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
| TC97 | P2 | Pending tx `blockHash` handling | `tx.blockHash` is `0x0` or `null` for pending |

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
| TC133 | P1 | eth_blockNumber + 1 == get_block("pending").number | Pending is one ahead |

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
- H_pending = pending hash (should be 0x0)
```

## Value derivation rules

### gasUsed
- Empty block: `gasUsed == 0`
- Block with txs: `gasUsed == sum(receipt.gasUsed for all txs)`
- Can cross-check with `eth_getBlockReceipts`

### size
- Sealed block: RLP-encoded size in bytes
- Pending block: 0 (implementation detail)
- Genesis block 0 has a specific size (e.g., 507 bytes per existing test)

### timestamp
- Must be Unix timestamp (seconds since epoch)
- Must be >= previous block's timestamp
- Should be close to actual wall clock time

### transactionsRoot, receiptsRoot, stateRoot
- Sealed: Merkle roots computed from block data
- Pending: `EMPTY_ROOT_HASH` = `0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421`

### logsBloom
- Sealed: Bloom filter from all logs in block
- Pending: All zeros (256 bytes)

## Test dependencies

- Use `setup_test_rollup(0, EVM_EXTENSION)` for instant finality
- Use `alloy_client(rollup.http_addr)` for Provider access
- Use `pause_preferred_batches()` / `resume_preferred_batches()` for determinism
- For EIP-1898 tests, may need raw JSON-RPC via `jsonrpsee`

## References

- [Ethereum JSON-RPC Specification](https://ethereum.org/developers/docs/apis/json-rpc/)
- [EIP-1898: Typed block parameter](https://eips.ethereum.org/EIPS/eip-1898)
- [QuickNode eth_getBlockByNumber](https://www.quicknode.com/docs/ethereum/eth_getBlockByNumber)
- [Alloy Provider Documentation](https://docs.rs/alloy-provider)
