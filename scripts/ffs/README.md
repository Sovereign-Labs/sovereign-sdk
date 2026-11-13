# Sequencer concurrency testing with FFS

The [CI workflow](../../.github/workflows/ffs.yml) runs a native baseline and ten FFS schedules of [`test_out_of_order_nonces`](../../crates/full-node/sov-sequencer/tests/integration/preferred_tx_nonce_queue.rs#L249). This is a narrow functional check, not a claim that the sequencer is concurrency-correct.

## CI operation

The workflow runs on relevant pull requests, pushes to `dev` or `ci/ffs-*`, and manual dispatch. It uses the privileged Namespace runner profile already used for native Rust tests. Default scheduling is `--ticks none`, which explores syscall interleavings without requiring a virtualized hardware counter. Manual dispatch can select `hw`; its counter self-test must pass, with no automatic fallback. A successful syscall-mode run does not establish hardware-mode coverage.

FFS is pinned to `986acb0de2589430b49621c4d4fb6bc94bfa869a` and built using Rust 1.94.1; SDK tests keep the repository toolchain. [The futex patch](replay-blocked.patch) allows a repeated blocked event to pass through normal replay processing. [The CI outcome patch](ci-outcomes.patch) makes depth exhaustion, internal errors, and divergences fail exploration rather than count as successful schedules. CI checks depth exhaustion explicitly before testing the sequencer.

The [replay diagnostic](reproduce-replay.sh) is advisory: it looks for known failures in the original FFS binary and checks the futex fix against the same trace. Failure to reproduce a timing-dependent tool issue does not fail sequencer assertions. Native baseline and exploration failures do fail the job. The new workflow is not added to the Rust workflow's aggregate required check; evaluate runner reliability before making it required.

Logs retain the seed, test results, diagnostic failures, and scheduling mode. Raw traces are not uploaded: FFS stores its entire process environment in them. Exploration begins at process start because live `io_uring` rings cannot be checkpointed. Full sequencer replay remains unreliable even after the futex fix, so replay divergence alone is not a sequencer defect.

## Local execution

Build the pinned FFS checkout, preserve an unpatched binary, apply both patches, and build again. Set `FFS_BIN` to the patched executable and `FFS_OUTPUT_DIR` to an absolute output directory. Then run:

```bash
SDK_ROOT=/absolute/path/to/sovereign-sdk
export FFS_BIN=/absolute/path/to/patched/ffs FFS_OUTPUT_DIR=/tmp/ffs-sequencer
SKIP_GUEST_BUILD=1 SP1_SKIP_PROGRAM_BUILD=1 FFS_MODE=explore FFS_TICKS=none FFS_ITERATIONS=10 \
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="bash $SDK_ROOT/scripts/ffs-sequencer-runner.sh" \
  cargo nextest run --manifest-path "$SDK_ROOT/Cargo.toml" \
  -p sov-sequencer --test integration -E 'test(=preferred_tx_nonce_queue::test_out_of_order_nonces)' --no-capture
```

## Follow-up plan

1. **Establish CI reliability.** Confirm native and FFS success on the actual runner, retain ten completed schedules per run, and measure runtime. Keep infrastructure failures distinct from assertion failures. Only make the workflow required after repeated clean runs; retain explicit failures for incomplete exploration.
2. **Expand nonce coverage.** Add [`test_failed_transaction_aborts_drain`](../../crates/full-node/sov-sequencer/tests/integration/preferred_tx_nonce_queue.rs#L267), [`test_nonce_queue_timeout`](../../crates/full-node/sov-sequencer/tests/integration/preferred_tx_nonce_queue.rs#L330), and [`test_zero_length_queue`](../../crates/full-node/sov-sequencer/tests/integration/preferred_tx_nonce_queue.rs#L367). Start with three schedules each, then increase to ten. Check successful transaction order, rejection of failed or expired requests, final state, and termination of every request task.
3. **Test publication ordering.** Run the [delayed-checkpoint regression](../../crates/full-node/sov-sequencer/tests/integration/preferred_end_to_end.rs#L580) under schedule exploration. Check that state is queryable after its corresponding notification. Add a small concurrent submit/query scenario only if existing assertions do not cover confirmation visibility; do not assume HTTP and WebSocket network delivery order.
4. **Test recovery and shutdown.** Start with [blob publication after resync](../../crates/full-node/sov-sequencer/tests/integration/preferred_blob_sender.rs#L179), then [restart tests](../../examples/demo-rollup/tests/restart/graceful.rs#L279) and [small reorg scenarios](../../crates/full-node/sov-sequencer/tests/integration/preferred_with_reorgs.rs#L472). The linked reorg test is currently ignored; first establish a passing native baseline and explicitly select ignored tests. Assert bounded shutdown, recovery of accepted transactions according to the durability contract, consistent state after catch-up, and eventual DA publication. Actual crash durability needs explicit crash/restart injection; scheduling alone is insufficient.
5. **Check execution against canonical replay.** Exercise [background state-root computation](../../crates/full-node/sov-sequencer/src/preferred/state_root_compute.rs#L291) while the node advances and [final catch-up](../../crates/full-node/sov-sequencer/src/preferred/sync_sequencer_state/sync_state.rs#L751) while submissions continue. Compare user roots and receipts for the same executed history. Do not compare arbitrary concurrent request orders as if they must produce identical state.
6. **Resolve tool limitations before scaling.** Use [the standalone reproducers](replay-repro.c) to investigate the remaining `io_uring` divergence, and reduce the hardware-tick mismatch separately. Validate any tool change against both its reproducer and the original sequencer trace. Propose upstream reports separately. Once replay is dependable, increase seeds and add a scheduled campaign with a measured runtime budget.

For every suspected sequencer failure, preserve the exact SDK and FFS revisions, compiler versions, scheduling mode, seed, logs, and local trace. Classify it as an assertion failure, progress failure, tool divergence, or unsupported runner capability before changing production code. A passing sample of schedules increases coverage; it does not prove the absence of concurrency bugs.
