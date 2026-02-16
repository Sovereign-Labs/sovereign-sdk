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


# Runner-Side Ordering Issue
The issue is a missing ordering guarantee between:

1. API checkpoint update (checkpoint_receiver path), and
2. slot notification emission (api_ledger_db path).

Today these happen on different async paths without an ack barrier.

Where it happens

1. Sequencer sync path enqueues checkpoint update event, then immediately sends ledger notifications:

- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:660
- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:666
- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:670

2. force_update_api_state is only an async send to event queue:

- crates/full-node/sov-sequencer/src/preferred/executor_events.rs:184

3. Side-effects task applies checkpoint later when event is processed:

- crates/full-node/sov-sequencer/src/preferred/side_effects.rs:214
- crates/full-node/sov-sequencer/src/preferred/side_effects.rs:215
- Queue is batched/drained and can delay this event under load:
- crates/full-node/sov-sequencer/src/preferred/side_effects.rs:246
- crates/full-node/sov-sequencer/src/preferred/side_effects.rs:261

4. Ledger notifications are sent immediately in update_api_ledger:

- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/inner.rs:456
- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/inner.rs:470

5. REST builds accessor from current checkpoint snapshot; stale checkpoint can yield HeightNotAccessible:

- crates/module-system/sov-modules-api/src/rest/mod.rs:236
- crates/module-system/sov-modules-api/src/state/accessors/http_api.rs:696
- mapped to HTTP 404 "invalid rollup height":
- crates/module-system/sov-modules-api/src/rest/mod.rs:370
- crates/module-system/sov-modules-api/src/state/accessors/http_api.rs:530

6. Same pattern also exists in recovery overwrite flow:

- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:703
- crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs:706

Concrete timeline

1. New node state info arrives.
2. Sequencer enqueues ForceUpdateApiState(checkpoint) (not applied yet).
3. Sequencer immediately replaces ledger reader and emits slot notifications.
4. Client receives slot WS update and queries historical state.
5. REST still sees previous checkpoint and returns HeightNotAccessible => 404.
6. Shortly after, side-effects applies checkpoint and same query succeeds.

This is exactly a transient API visibility gap.

———

On the reviewer concerns
Both concerns are correct.

1. highest_accessible_rollup_height can mask immediate-availability failures

- In current test, inaccessible latest heights are skipped:
- crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs:3597
- That means the test can pass while most recent notified heights are still not queryable.

2. Retries changed semantics from “immediate” to “eventual”

- Current test retries on 404/HeightNotAccessible:
- crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs:3581
- crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs:3619
- So it now tolerates propagation delay and does not strictly enforce “immediate”.

So yes: current version is better as an eventual consistency + immutability test, not a strict immediate-availability contract test.

———

What runner-side hardening should do
The core fix is to create a happens-before guarantee:

1. Add an acked checkpoint-update event (oneshot ack from side-effects after checkpoint_sender.send).
2. In sync path, await this ack before update_api_ledger(...send_notifications_for_slot...).
3. Apply same ordering in recovery path.

After that, slot notification implies checkpoint is already updated for API queries.


```
 Context

 There is a race condition in the sequencer's API state update path. When new node state arrives, the sync path:
 1. Enqueues a checkpoint update via mpsc to the side-effects task
 2. Immediately sends slot notifications to WebSocket clients

 The side-effects task applies the checkpoint to a watch::Sender asynchronously. If a client receives the WS notification and queries the REST API before the checkpoint is applied, the query fails with HeightNotAccessible (404).

 This is a transient API visibility gap — the same query succeeds moments later. It affects three code paths in sync_state.rs: common_for_final_catchup_and_new_storage, process_force_overwrite_state_for_recovery, and
 process_wait_for_node_resync.

 Race timeline:
 SyncState                   SideEffects                 Client
   |                            |                           |
   |-- mpsc: ForceUpdateApi --> |                           |
   |-- broadcast: slot notif ---|-------------------------> |
   |                            |                           |-- REST query (404!)
   |                            |-- watch: apply checkpoint |
   |                            |                           |-- REST query (200 ok)
```