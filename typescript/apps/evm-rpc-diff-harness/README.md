# EVM RPC Differential Harness

Correctness-only differential test harness for comparing a standard EVM tooling view of:

- `ANVIL_RPC_URL` baseline
- `ROLLUP_RPC_URL` target

The harness deploys the same Solidity fixtures to both endpoints, runs the same checks via `ethers v6`, `viem`, and raw JSON-RPC, then writes:

- `artifacts/report.json` (machine-readable)
- `artifacts/report.md` (human-readable summary)

## Scope

This harness is focused on correctness and compatibility. It does **not** benchmark latency/throughput.

Checks implemented:

- A: Transaction lifecycle correctness (including EIP-1559 fallback behavior)
- A: CREATE2 + delegatecall + ETH transfer path correctness
- A: Revert behavior (`revert(string)`, custom error, panic)
- B: `eth_call` correctness for scalar and dynamic ABI types
- C: `eth_estimateGas` sanity and revert behavior
- D: `eth_getLogs` filtering and decoded content checks
- E: Block shape conformance + chain fields (`eth_chainId`, `net_version`, `web3_clientVersion`)
- F: JSON-RPC error convention checks (with explicit expected vs actual error payloads)
- G: Batch request support + response shape checks
- H: `eth_blockNumber` / latest block consistency
- I: Block-tag state-read semantics for `eth_call` (latest/pending/hex tags)
- J: Pending-to-sealed transaction/receipt transition checks
- K: `eth_getBlockByHash` + block transaction count API consistency
- L: `eth_feeHistory` shape and invalid-tag behavior
- M: `eth_getBlockReceipts` support/shape consistency

## Prerequisites

- Node.js 20+
- pnpm
- Anvil (`foundry`) for local baseline tests

## Install

From `typescript/apps/evm-rpc-diff-harness`:

```bash
pnpm i
```

Or from `typescript` workspace root:

```bash
pnpm i
```

## Usage

### 1) Compare an existing anvil endpoint and rollup endpoint

```bash
pnpm run compare -- --anvil $ANVIL_RPC_URL --rollup $ROLLUP_RPC_URL --pk $TEST_PRIVATE_KEY
```

You can also supply environment variables instead of flags:

- `ANVIL_RPC_URL`
- `ROLLUP_RPC_URL`
- `TEST_PRIVATE_KEY`
- Optional: `CHAIN_ID_ANVIL`, `CHAIN_ID_ROLLUP` (auto-detected if omitted)

### 2) Auto-start anvil baseline

```bash
pnpm test:anvil
```

This command:

- starts local anvil (default `http://127.0.0.1:8545`; override host/port with `ANVIL_RPC_URL`)
- prints funded test account details used by the harness
- runs compare against:
  - baseline: started anvil
  - rollup: `ROLLUP_RPC_URL` if set, otherwise same local anvil

`ANVIL_RPC_URL` in `test:anvil` mode must be an `http://` URL with host and optional port only (no path/query/hash).
By default, raw anvil logs are suppressed and only a failure tail is printed when the run fails. Set `ANVIL_VERBOSE_LOGS=1` to stream raw anvil logs.

## Outcome semantics

Each check is labeled with one outcome:

- `PASS`: rollup behavior matches compatibility expectation for that check
- `FAIL`: mismatch with baseline/shape expectations
- `NOT_SUPPORTED`: method/path unsupported; exact JSON-RPC error is recorded

For `FAIL`, the report includes normalized diffs plus explicit `Expected (Anvil)` and `Actual (Rollup)` payload blocks.
For `NOT_SUPPORTED`, the report includes exact captured `error.code`, `error.message`, `error.data`, and raw response payloads when present.

## Notes

- The harness normalizes volatile fields (`txHash`, `blockHash`, `blockNumber`, `timestamp`) before comparisons.
- It keeps shape/type conformance checks strict where applicable.
- The same private key must be funded on both endpoints.
