# Investigation: `flaky_test_archival_state_is_immediately_available`

## Test Under Investigation

- Test: `crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs:256`
- Assertion site: `crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs:312-320`
- Helper that merges failure modes into one `anyhow::Error`: `crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs:3500-3528`

The test assumes a strict mapping: each next rollup height increases `value-setter` value by exactly `+1`.

## Latest Revalidation (2026-02-09)

### Run Set A (outside sandbox, nextest with test retries)

- Command: `SKIP_GUEST_BUILD=1 cargo nextest run -p sov-sequencer flaky_test_archival_state_is_immediately_available`
- Result: 6/6 retries failed
- Failure type: all were value mismatch (`Condition failed: found_value == expected`)
- No `API request failed`, no 404 signal
- Log: `/tmp/flaky_reanalysis.log`

### Run Set B (50 independent single-shot executions)

- Command used repeatedly on compiled binary:
  - `env SKIP_GUEST_BUILD=1 timeout 25s target/debug/deps/integration-c9679bfbcc3cf732 --exact preferred_end_to_end::flaky_test_archival_state_is_immediately_available --nocapture`
- Runs: 50
- Pass: 5
- Fail: 45
- `API request failed` count: 0
- `HeightNotAccessible` count: 0
- `Condition failed` count: 45
- Delta check across all failures: `expected - found = 1` for all 45
- Log: `/tmp/flaky_binary_reanalysis.log`

### Instrumented Debug Run

- Command: `SKIP_GUEST_BUILD=1 RUST_LOG=debug cargo nextest run -p sov-sequencer flaky_test_archival_state_is_immediately_available --retries 0 --no-capture`
- Log: `/tmp/flaky_instrumented_reanalysis.log`

Key observations from this run:

- Multiple empty preferred batches are produced (`num_txs=0`) by sequencer and applied by node:
  - `/tmp/flaky_instrumented_reanalysis.log:183`
  - `/tmp/flaky_instrumented_reanalysis.log:205`
  - `/tmp/flaky_instrumented_reanalysis.log:216`
  - `/tmp/flaky_instrumented_reanalysis.log:272`
  - `/tmp/flaky_instrumented_reanalysis.log:285`
- Query sequence near failure shows flat value across two consecutive heights:
  - `rollup_height=10 -> value=6`: `/tmp/flaky_instrumented_reanalysis.log:1109-1114`
  - `rollup_height=11 -> value=6`: `/tmp/flaky_instrumented_reanalysis.log:1116-1121`
- Failure then occurs at `past height = 11, expected value = 7`:
  - `/tmp/flaky_instrumented_reanalysis.log:1122`
- HTTP status remains 200 in this failing path (not 404):
  - `/tmp/flaky_instrumented_reanalysis.log:1118-1119`

## Confirmed Root Cause For This Test

## 1) Empty Preferred Batches Shift Height-to-Value Mapping

### Why this is happening

After startup, updates take the simple path (`do_simple_state_update`) and run prune (`process_prune_sequencer_db`).
Prune triggers opportunistic production (`trigger_batch_production_if_convenient`), which can:

1. start a batch (this increments rollup height), then
2. close immediately with 0 transactions.

Relevant paths:

- `crates/full-node/sov-sequencer/src/preferred/update_state.rs:234-249`
- `crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:673-677`
- `crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/inner.rs:357-410`
- `crates/full-node/sov-sequencer/src/preferred/block_executor.rs:905`

### Effect on this test

The test expectation is index-based (`expected_value = j + 1`) rather than derived from actual state transitions.
When an empty batch inserts a height with no value change, all subsequent expectations are shifted by +1, so assertion sees `found = expected - 1`.

This exactly matches the 50-run distribution (all 45 failures had delta 1).

## Important Clarification On The “Stale Checkpoint Race”

There is a real ordering concern in code:

- `force_update_api_state(checkpoint)` is enqueued through mpsc
- then `update_api_ledger(...).send_notifications_for_slot(...)` runs immediately

Reference:

- `crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:646-671`
- `crates/full-node/sov-sequencer/src/preferred/executor_events.rs:184-188`
- `crates/full-node/sov-sequencer/src/preferred/side_effects.rs:214-216`
- `crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/inner.rs:456-478`

However, in the fresh revalidation above, this test did **not** reproduce 404/`HeightNotAccessible`.
For this specific flaky test, current evidence points to empty-batch height shifts as the dominant cause.

### Additional correction to prior writeup

The prior claim that archival user/kernel reads are generally unbounded was too broad.
In archival mode, `ApiStateAccessor` uses historical APIs for all namespaces:

- User: `get_historical::<User>`
- Kernel: `get_historical::<Kernel>`
- Accessory: `get_accessory_historical`

Reference: `crates/module-system/sov-modules-api/src/state/accessors/http_api.rs:115-151`, `crates/module-system/sov-modules-api/src/state/accessors/http_api.rs:473-523`.

So stale-checkpoint behavior is still possible via initialization timing/order, but it is not needed to explain the observed flake pattern in this test.

## Recommended Path

1. Fix the test expectation model first (highest-confidence fix):
   - Stop assuming one value increment per height index.
   - Derive expected value per queried height from observed slot events / actual committed state.
2. Optional for deterministic isolation:
   - Disable opportunistic automatic batch production for this test scenario.
3. Track checkpoint-vs-notification ordering as a separate issue:
   - Good production hardening, but currently not required to resolve this flake.
