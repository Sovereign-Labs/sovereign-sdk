# Description

Backend agnostic metrics tracking for Sovereign Rollups.

This crate provides a flexible and backend-agnostic framework for tracking custom metrics in Sovereign Rollups.
The primary interface allows developers to define and record metrics that are serialized in
the [Telegraf line protocol](https://docs.influxdata.com/influxdb/cloud/reference/syntax/line-protocol/).
Metrics are timestamped automatically and can only be tracked in **native mode**.

## Architecture overview

### **Tracker**

The Tracker is responsible for recording metrics and associating them with a timestamp.
It collects the data and forwards it to the Publisher for processing.

### **Publisher**

The Publisher buffers incoming metrics and efficiently sends them to Telegraf.
For optimal performance, metrics are serialized and published in a background thread,
ensuring minimal impact on the main application thread.

For a more detailed overview of the entire observability stack and its integration with Grafana, refer to this tutorial:
[Tutorial about observability](https://sovlabs.notion.site/Tutorial-Getting-started-with-Grafana-Cloud-17e47ef6566b80839fe5c563f5869017?pvs=74)

## Defining Custom Metrics

To define a custom metric, follow these steps:

1. Create a struct representing your metric. The struct can include any number
   of [fields and tags](https://docs.influxdata.com/influxdb/v1/concepts/key_concepts/).
2. Implement the [`Metric`] trait for your struct. This implementation should serialize metrics using the
   Telegraf line protocol format.
   > **Note**: Be mindful of special characters in metric names, field names, and tag names. You do **not** need to
   > explicitly record the metric's timestamp—this is handled by the `sov_metrics` crate.

### Example

Below is an example of defining a custom metric:

```rust
use std::io::Write;

#[derive(Debug)]
pub struct MyCustomMetric {
    time: std::time::Duration,
    value: u64,
    tag: u64,
}

impl sov_metrics::Metric for MyCustomMetric {
    fn measurement_name(&self) -> &'static str {
        "my_custom_metric"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} my_tag={} my_value={},my_time_spent_ms={}",
            self.measurement_name(),
            self.tag,
            self.value,
            self.time.as_millis(),
        )
    }
}
```

In this example:

- `MyCustomMetric` tracks a time duration (`time`), a numerical value (`value`), and a tag (`tag`).
- The `serialize_for_telegraf` method ensures the metric is properly formatted for tracking.

## Tracking Metrics

To track metrics, use the [`track_metrics`] function and pass a closure that contains your metrics logic.
Metrics can only be tracked when the code is compiled with the `native` feature flag enabled.

### Example

Here's an example of tracking metrics during the execution of some computational logic:

```rust
# #[derive(Debug)]
# struct MyCustomMetric {
#     time: std::time::Duration,
#     value: u64,
#     tag: u64,
# }
#
# impl sov_metrics::Metric for MyCustomMetric {
#     fn measurement_name(&self) -> &'static str {
#         "a"
#     }
#
#     fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
#        use std::io::Write;
#         write!(
#             buffer,
#             "{} tag={} v={},time={}",
#             self.measurement_name(),
#             self.tag,
#             self.value,
#             self.time.as_millis(),
#         )
#   }
# }

fn my_code(input: u64) -> u64 {
    sov_metrics::start_timer!(start_operation);
    let result: u64 = some_expensive_operation(input);
    sov_metrics::save_elapsed!(my_operation_time SINCE start_operation);
    #[cfg(feature = "native")]
    {
        sov_metrics::track_metrics(|tracker| {
            // Timestamp will be added at this moment.
            tracker.submit(
                MyCustomMetric { value: result, tag: input, time: my_operation_time }
            );
            // More metrics can be tracked at once
        })
    }
    result
}


fn some_expensive_operation(input: u64) -> u64 {
   input * 2
}
```

### Key Points:

- The `track_metrics` function records all metrics during the function's execution.
- Timestamping is handled automatically when metrics are being tracked.

## Metric inventory

All measurement names emitted by the SDK are listed below. Every name is prefixed with
`sov_` for discoverability in InfluxDB / Grafana. New metrics should follow the same
convention; prefer the `sov_rollup_`, `sov_nomt_`, `sov_db_`, `sov_evm_`, `sov_sequencer_`,
`sov_hyperlane_`, `sov_proxy_`, or `sov_celestia_adapter_` namespaces that match the
emitting subsystem.

The "What to look for" column is aimed at both human operators and autonomous agents
diagnosing a running rollup: it names the dominant diagnostic signal in each metric and
points at the metric's usual partners when the signal is ambiguous. Numeric thresholds
are rules of thumb, not hard limits.

### Runner / STF

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_rollup_runner_da` | gauge | `sov-metrics/src/influxdb/tracker.rs` | `sync_distance` trending up → node falling behind DA tip; `get_block_time_ms` spiking → DA RPC slow or unreachable. |
| `sov_rollup_runner_counts` | counter | `sov-metrics/src/influxdb/tracker.rs` | Rate of `batches`, `transactions`, `proofs_processed` per DA height. Sustained zeros on a public network = sequencer starvation or DA extraction bug. |
| `sov_rollup_runner_times_us` | timer | `sov-metrics/src/influxdb/tracker.rs` | Which stage dominates: `apply_slot`, `stf_transition`, `extract_blobs`, `processing_changes`. The bottleneck identifies which subsystem to investigate next. |
| `sov_runner_process_stf_changes` | timer | `sov-metrics/src/influxdb/tracker.rs` | Post-STF pipeline (finalization, ledger materialization, prover dispatch, API push). `sending_stf_to_prover_time` high → prover queue/IO backlog. |
| `sov_rollup_transaction_execution_us` | timer | `sov-metrics/src/influxdb/tracker.rs` | Per-tx latency tagged by `status`/`context`/`call_message`/`sequencer`. Group by tag to find expensive message types or misbehaving sequencers. ⚠️ See cardinality note below. |
| `sov_rollup_slot_execution_time_us` | timer | `sov-metrics/src/influxdb/tracker.rs` | End-to-end slot latency; the top-level SLO for rollup throughput. If this grows, drill into `sov_rollup_runner_times_us` + `sov_storage_manager_finalization`. |
| `sov_rollup_batch_processing` | timer + counter | `sov-metrics/src/influxdb/tracker.rs` | Per-batch cost and tx count; use together to compute per-tx cost trends inside a batch. |
| `sov_rollup_auth_and_process_metrics` | timer | `sov-metrics/src/influx_db_nonnative.rs` | Authentication + processing cost (emitted in non-native / ZK-guest code paths). Budgets ZK cycle consumption. |

### HTTP / RPC surface

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_rollup_rpc_handlers` | timer + status | `sov-metrics/src/influxdb/tracker.rs` | Slow RPC methods (tag `request_name`) and error rates (tag `status`). Sudden error bursts with low latency = validation failures; slow + errors = downstream dependency. |
| `sov_rollup_http_handlers` | timer + status | `sov-metrics/src/influxdb/tracker.rs` | Same pattern for REST. ⚠️ The `path` tag currently uses raw `request_uri.path()`; paths containing IDs explode cardinality. Normalization is tracked in PR #2753. |

### ZK VM

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_rollup_zkvm` | cycles + memory | `sov-metrics/src/influxdb/tracker.rs` | Cycles / heap usage per named call site in the guest. Use to find hot functions blowing up circuit size. `name` is the only tag (bounded — one per `#[cycle_tracker]`-annotated function); caller-supplied `metadata` is emitted as string fields, so it stays cardinality-safe even when call sites pass hashes or heights. |
| `sov_rollup_zkvm_proving` | timer | `sov-metrics/src/influxdb/tracker.rs` | Proof generation wall-clock per `circuit`. Success/failure ratio via `is_success`. |
| `sov_nomt_prover_compute_state` | counter | `sov-state/src/nomt/prover_storage.rs` | Prover-side state computations, split by `with_witness`. |

### Runtime / infrastructure

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_rollup_tokio_runtime` | runtime | `sov-metrics/src/influxdb/tracker.rs` | Tokio worker saturation and scheduling. Busy workers pegged near total workers = runtime is CPU-bound; high poll latencies = blocking code on async thread. |
| `sov_rollup_dropped_metrics` | counter | `sov-metrics/src/influxdb/tracker.rs` | **Any non-zero sustained value means the metrics pipeline is backpressured** — Telegraf or the publisher task can't keep up. Other metrics become unreliable until this is zero again. |
| `sov_rollup_rate_limiter` | timer + counter | `sov-metrics/src/influxdb/tracker.rs` | Rate limiter hits by `limiter_type`. Non-zero = clients throttled; cross-check against `sov_rollup_rpc_handlers` / `sov_rollup_http_handlers` 429 responses. |
| `sov_rollup_gas_constant` | counter | `sov-metrics/src/influxdb/gas_constant_estimation.rs` | Empirical gas cost samples keyed by `name`/`constant` (both bounded tags). Only emitted with the `gas-constant-estimation` feature. Caller-supplied `metadata` is emitted as string fields, so it stays cardinality-safe even when call sites pass hashes or heights. |

### Module implementations

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_evm_tx` | timer | `sov-evm/src/metrics.rs` | EVM tx timing breakdown (`fetch_state`, `execution`, `state_commit`, `receipt`, `get_head`). Which stage dominates identifies whether the bottleneck is revm, state I/O, or commit. |
| `sov_evm_db_metrics` | timer + counter | `sov-evm/src/db/metrics.rs` | revm DB access counts and durations per access type (`account`, `code`, `storage`, `block_hash`). High `storage_count` = tx is doing many slot reads. |
| `sov_rollup_value_setter` | timer | `sov-synthetic-load/src/metrics.rs` | Only emitted in synthetic-load benchmarks (`sov-synthetic-load`); tag `context` distinguishes the workload shape. Ignore in production. |
| `sov_hyperlane_rate_limiter_capacity` | gauge | `hyperlane/src/warp/metrics.rs` | Current and max rate-limiter capacity by route, remote domain, and direction. Watch `current_capacity` near zero for throttled bridge traffic. |

### Storage (sov-db / NOMT)

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_state_db_materialization` | counter + size | `sov-db/src/metrics/mod.rs` | `max_value_size` jumping = a module wrote a large blob; `cumulative_values_size` climbing without matching tx volume = state bloat. Correlate with `sov_rollup_slot_execution_time_us`. |
| `sov_nomt_db_stats` | cache | `sov-db/src/metrics/nomt.rs` | `hash_table_occupied / hash_table_capacity > 0.9` → NOMT warns + perf degrades, need resync with larger capacity. `page_cache_misses / page_requests` high → insufficient RAM/cache. |
| `sov_nomt_begin_session` | timer | `sov-db/src/metrics/nomt.rs` | `overlays` climbing = finalization is lagging. `init_time` spiking on its own = storage engine contention. Tag `db` splits by instance. |
| `sov_storage_manager_finalization` | timer | `sov-db/src/metrics/nomt.rs` | `commit_time` dominating slot cost = storage is the bottleneck. `pruning_commit_time` repeatedly large = pruner backlog (see `sov_db_pruner`). |
| `sov_db_pruner` | counter + timer | `sov-db/src/metrics/nomt.rs` | `keys_to_prune` near zero while `keys_inspected` is non-zero = wasted scans (retention misconfigured?). No emissions over long windows = pruner task may be stuck. |
| `sov_nomt_commit_detailed` | timer | `sov-db/src/metrics/nomt.rs` | Breaks NOMT commit into write_user / write_kernel / flat / accessory / ledger phases. Use to attribute slow `sov_storage_manager_finalization.commit_time` to a specific phase. |

### Sequencer (preferred role)

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_rollup_current_sequence_number` | gauge | `sov-sequencer/src/metrics.rs` | Must increase monotonically. Flat for extended periods while the rollup is live = sequencer stalled. |
| `sov_rollup_in_progress_batch_size` | gauge | `sov-sequencer/src/metrics.rs` | Growing unboundedly = sequencer is accumulating but not producing batches; cross-check `sov_rollup_preferred_sequencer_channel`. |
| `sov_rollup_preferred_sequencer_update_state` | timer + counter | `sov-sequencer/src/metrics.rs` | Main state-update loop cost. `total_message_processing_duration` high = event flood or slow event handlers (see `sov_rollup_preferred_sequencer_executor_event`). |
| `sov_rollup_preferred_sequencer_channel` | timer | `sov-sequencer/src/metrics.rs` | Blocking time on the channel send, tagged by `reason`. High = downstream consumer is slow; consumer identified by the reason tag. |
| `sov_rollup_preferred_sequencer_executor_event` | timer | `sov-sequencer/src/metrics.rs` | Per-event-type handling time; tag `event_type` is low-cardinality so safe to group by. |
| `sov_rollup_preferred_sequencer_fetch_batches_to_replay` | timer + counter | `sov-sequencer/src/metrics.rs` | Replay fetch on rebase/restart. Spikes here map to rebase windows; see `STATE_ROOT_DELAY_BLOCKS`. |
| `sov_rollup_preferred_sequencer_slot_numbers` | gauge | `sov-sequencer/src/metrics.rs` | Four slot-number views (`true`, `latest_finalized`, `node_visible`, `seq_visible`). Divergence between `seq_visible` and `node_visible` → sequencer and node disagree on visibility. |
| `sov_rollup_preferred_sequencer_prune` | timer | `sov-sequencer/src/metrics.rs` | Internal sequencer prune (different from DB pruner). Large values here don't directly affect txs but grow memory. |
| `sov_rollup_preferred_sequencer_executor_event_sending` | timer | `sov-sequencer/src/metrics.rs` | Send-side blocking for executor events. `blocked_for_us` non-zero = executor is backpressuring the sequencer. |
| `sov_rollup_nonce_buffer_main_queue_blocked` | timer | `sov-sequencer/src/metrics.rs` | **Only emitted when send blocked** (`blocked_for_us > 0`); presence of this metric = main queue capacity pressure. |
| `sov_rollup_nonce_buffer_main_queue_depth` | gauge | `sov-sequencer/src/metrics.rs` | Instantaneous main queue depth. Compare against configured capacity to spot near-full saturation. |
| `sov_rollup_nonce_buffer_timeout_queue` | timer + gauge | `sov-sequencer/src/metrics.rs` | Timeout queue activity (txs parked waiting for their nonce to become current). Deep queue = out-of-order nonces from clients. |
| `sov_rollup_sequence_number_delta` | gauge | `sov-sequencer/src/metrics.rs` | Gap between expected and observed sequence numbers. Non-zero briefly is normal during rebase; sustained non-zero = desync. |
| `sov_sequencer_cache_warmup_metrics` | gauge | `sov-sequencer/src/preferred/cache_warm_up_executor.rs` | Tx-channel size seen by the cache warm-up executor; use as a sanity check that warm-up is receiving work. |

### Blob sender

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_rollup_in_flight_blobs_snapshot` | snapshot | `sov-blob-sender/src/metrics.rs` | Per-blob lifecycle snapshot. `duration_ms` high with stable `last_known_state` = blob stuck in that state (DA submission hanging or resurrection loop). |
| `sov_rollup_num_of_in_flight_blobs` | gauge | `sov-blob-sender/src/metrics.rs` | Growing unboundedly = DA submission is not keeping up with blob production. |
| `sov_rollup_blobs_enter_scope` | counter | `sov-blob-sender/src/metrics.rs` | Rate of new blobs being handed to the sender. Compare with `exit_scope` for throughput balance. |
| `sov_rollup_blobs_exit_scope` | counter | `sov-blob-sender/src/metrics.rs` | Rate of blobs leaving the sender (success or drop). `enter - exit` over a window ≈ backlog growth. |

### Celestia adapter

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_celestia_adapter_header_get_by_height` | timer + status | `celestia/src/metrics/client.rs` | Celestia header fetch RPC; `is_success=false` = node down or misconfigured. Elevated latency = bridge/RPC slow. |
| `sov_celestia_adapter_header_network_head` | timer + status | `celestia/src/metrics/client.rs` | Polling for network head; similar failure-mode semantics as `header_get_by_height`. |
| `sov_celestia_adapter_share_get_namespace_data` | timer + status | `celestia/src/metrics/client.rs` | Data-share retrieval per namespace. Failures here often surface upstream as `sov_rollup_runner_da` gaps. |
| `sov_celestia_adapter_state_submit_pay_for_blob` | timer + status | `celestia/src/metrics/client.rs` | PFB submission to Celestia; failures block DA posting entirely. Pair with `sov_rollup_in_flight_blobs_snapshot` to confirm blobs are stuck here vs. elsewhere. |
| `sov_celestia_adapter_blob_get_all` | timer + status | `celestia/src/metrics/client.rs` | Blob lookup RPC; failures mean submitted blobs cannot be fetched back from Celestia. |
| `sov_celestia_adapter_state_balance_for_address` | timer + status | `celestia/src/metrics/client.rs` | Balance lookup for the Celestia account; failures block balance-aware health checks and funding diagnostics. |
| `sov_celestia_adapter_header_sync_state` | timer + status | `celestia/src/metrics/client.rs` | Header sync-state RPC; failures or high latency make DA-head visibility unreliable. |
| `sov_celestia_adapter_state_estimate_gas_price` | timer + status | `celestia/src/metrics/client.rs` | Gas-price estimation RPC; failures can prevent cost-aware PFB submission. |
| `sov_celestia_adapter_get_block` | timer | `celestia/src/metrics/full.rs` | Block-level fetch latency (full-node path). `height` and `square_width` are fields (not tags); `square_width` is a useful indicator of on-chain activity. |
| `sov_celestia_adapter_submit_blob` | timer | `celestia/src/metrics/full.rs` | Full-node blob submission path; tag `namespace` is low-cardinality. |
| `sov_celestia_adapter_periodic_data` | gauge | `celestia/src/metrics/full.rs` | Periodic adapter health snapshot: balance, gas price, and sync distance. Low balance or growing sync distance points to operator intervention. |

### Proxy utilities

| Name | Kind | Defined in | What to look for |
|---|---|---|---|
| `sov_proxy_latest_height_check` | gauge | `sov-proxy-utils/src/node_check_metric.rs` | Cross-node latest-height health: `nodes_failed` non-zero or `height_diff` widening means the proxy pool is inconsistent. |
| `sov_proxy_root_hash_check` | gauge | `sov-proxy-utils/src/node_check_metric.rs` | Cross-node state-root consistency at a slot. `unique_state_roots > 1` is a consensus-critical disagreement signal. |
| `sov_proxy_cluster_update_failure` | counter | `sov-proxy-utils/src/node_discovery_metrics.rs` | Node-discovery refresh failures by `stage`; sustained increments mean proxy membership is stale. |

### Known cardinality caveats

Grafana / Flux queries that group by high-cardinality tags can blow out InfluxDB memory.
Known cases today:

- `sov_rollup_http_handlers` — `path` is the raw `request_uri.path()`; paths containing
  IDs (`/blocks/12345`, `/tx/0xabc…`) create one series per ID. Normalization is tracked
  in PR #2753.
- `sov_hyperlane_rate_limiter_capacity` — series count is `monitored_routes × enrolled_domains × 2`.
  Cardinality is operator-controlled via `WarpExecutionConfig.monitored_routes`; a large
  monitored list × many enrolled destinations can still pressure InfluxDB. Prefer enumerating
  only the routes you actively care about.

Note: `sov_rollup_zkvm` and `sov_rollup_gas_constant` used to carry caller-supplied
`metadata` as tags. Those are now emitted as string fields instead, so enabling the
`bench` or `gas-constant-estimation` features against a real InfluxDB no longer risks
series explosion.
