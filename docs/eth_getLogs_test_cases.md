# eth_getLogs test cases

## Summary
- Returns logs that match a filter, scoped by block range or block hash.
- Logs are ordered by block number, transaction index, then log index.
- This rollup uses soft-confirmation semantics: `latest` == `pending`, and logs are expected to be visible immediately.

## Parameters (filter object)
- `fromBlock` / `toBlock`: block tag or hex number; inclusive range.
  - Defaults: if omitted, assume `latest` (which equals `pending` for this rollup).
- `blockHash`: 32-byte hash; mutually exclusive with `fromBlock`/`toBlock`.
- `address`: single address or array of addresses (OR semantics).
- `topics`: array (len <= 4). Each position may be:
  - `null` (wildcard),
  - a single topic,
  - an array of topics (OR semantics for that position).

## Rollup-specific semantics (assumptions)
- `latest` and `pending` return the same soft-confirmed view.
- Logs from the soft-confirmed block include a synthetic `blockHash`, and that hash is queryable via `blockHash`.
- `safe` and `finalized` map to the last sealed (canonical) head and should exclude pending logs (confirm if needed).
- `removed` is always `false` in `eth_getLogs` responses.
- Error-shape and invalid-param tests are out of scope for this PR.

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
