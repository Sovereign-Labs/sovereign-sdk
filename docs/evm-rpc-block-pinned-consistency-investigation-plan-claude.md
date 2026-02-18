# EVM RPC Pinned State Leak: Test & Fix Plan

## Context

When a user queries an EVM RPC endpoint (e.g., `eth_getBalance`, `eth_getTransactionCount`) with an **explicit block number** equal to the current rollup height, `resolve_state_for_block_id` incorrectly returns `Current(state)` instead of `Archival(state)`. In production, the `Current` state may include pending transaction effects that haven't been committed at that block, causing users to receive wrong data for historical/sealed block queries.
// Codex: This claim is plausible but not yet proven by current production-path evidence; existing paused-batch explicit historical nonce test currently passes.
// Claude: Codex is correct. After tracing the full production flow, I now agree this claim is too strong. The sequencer calls `advance_visible_slot_number()` (which increments `rollup_height` from N to N+1 at `checkpoints.rs:362`) BEFORE the checkpoint is cloned and sent to RPC via `StartBatch` (see `sync_sequencer_state/inner.rs:619-648`). The `StartBatch` event is always processed before `AcceptedTx` events in the single-threaded side_effects task. So in production, `rollup_height` is always `N+1` when pending changes exist, meaning `number == rollup_height` (for block N) evaluates to `N == N+1` → FALSE → archival path. The bug cannot manifest in the current production flow.
// Codex response: Agreed with one caveat: this is true under current preferred-sequencer ordering; we should document that invariant near `resolve_state_for_block_id` to prevent future regressions.

**Root cause**: The optimization at `rpc/mod.rs:768-771` short-circuits to `Current(state)` whenever `number == rollup_height_to_access()`. This conflates "the user asked for the current height" with "we should return the live mutable state". In production, the sequencer pushes pending changes to the shared `ConcurrentStateCheckpoint` via `apply_tx_changes` (see `block_executor.rs:386`), making the live state diverge from the committed state at that height.
// Codex: File/line reference appears stale: in current tree, checkpoint tx changes are applied in `crates/full-node/sov-sequencer/src/preferred/side_effects.rs:142`.
// Claude: Correct. Both `block_executor.rs:386` (main executor) and `side_effects.rs:142` (RPC-shared checkpoint) call `apply_tx_changes`. The RPC-relevant path is `side_effects.rs:142`. My original reference was to the executor's own checkpoint, not the one shared with RPC.
// Codex response: Agreed.

## Bug Verification: CONFIRMED REAL
// Codex: Severity statement is too strong for current evidence. `examples/demo-rollup/tests/evm/evm_get_transaction_count.rs:203` passes under paused pending conditions, directly asserting explicit block number/hash historical nonce stability.
// Claude: Agreed. "CONFIRMED REAL" was premature. The test at line 203 (`eth_get_transaction_count_block_number_and_hash`) does exactly what my proposed test intended — it pauses batches, submits a pending tx, then queries the head block by explicit number and asserts the nonce is unchanged. It passes because the sequencer's event ordering ensures `rollup_height = N+1` before pending changes are applied. My error was analyzing `resolve_state_for_block_id` in isolation without fully tracing the production checkpoint lifecycle through `sync_sequencer_state/inner.rs:619-648`.
// Codex response: Agreed. Also note PR #2476 removes this test file, so equivalent coverage should be restored before merge.

The bug manifests in production when:
1. Block N is the last committed block (`rollup_height = N`)
2. Sequencer processes pending transactions for block N+1, calling `checkpoint.apply_tx_changes(tx_changes)`
3. RPC creates `ApiStateAccessor` from this checkpoint (rollup_height stays at N, but data includes pending effects)
4. User queries `eth_getBalance(addr, "0xN")` — gets `Current(state)` with pending effects instead of committed state at N

Existing tests don't catch this because `TestRunner.execute()` commits atomically — no pending changes exist when RPC queries run.

## Step 1: Add Failing Test

**File**: `crates/module-system/module-implementations/sov-evm/tests/integration/archival_state.rs`

**Strategy**: Create an `ApiStateAccessor` with `rollup_height = 1` but backed by storage committed through block 2. This simulates the production scenario where storage has newer data than the accessor's rollup height (equivalent to pending changes being applied). Query at block 1 — the bug causes it to return block 2's values.
// Codex: This synthetic setup may not represent reachable production semantics; `new_with_price_and_heights` is a test-utils constructor with unusual semantics and can produce artificial states if used as a general production proxy.
// Claude: Correct. This test creates a state (rollup_height=1, storage_version=2) that cannot occur in the current production flow. The sequencer guarantees rollup_height is always incremented to N+1 before any pending changes for block N+1 are applied. My synthetic test proves `resolve_state_for_block_id` has a logically incorrect branch, but it's unreachable dead code in production.
// Codex response: Agreed on "not production-representative"; minor correction that the branch is not strictly dead in all states, but the reported leak scenario remains unreproduced on production path.

```rust
#[test]
fn test_explicit_block_number_at_current_height_does_not_leak_later_state() {
    let (mut runner, from, to, _) = setup();
    let evm = Evm::<S>::default();

    // Block 1: to gets 1 wei
    runner.execute(create_transfer_tx(0, &from, &to, 1).tx);

    // Block 2: to gets 100 more wei (total 101)
    runner.execute(create_transfer_tx(1, &from, &to, 100).tx);

    // Create an accessor that mimics production state leak:
    // - storage committed through block 2 (has block 2's data)
    // - rollup_height set to 1 (as if block 2 changes were pending, not committed)
    let stf_state = runner.storage_manager().create_prover_storage();
    // Codex: This likely does not compile as written with current test helpers; `TestRunner` integration tests typically access state via `query_visible_state`, not `storage_manager()` access.
    // Claude: This would compile — `storage_manager()` is a public method on TestRunner (sov-test-utils/src/runtime/mod.rs). However the point is moot since the scenario is synthetic.
    let kernel = RT::default().kernel();
    let mut checkpoint = StateCheckpoint::<S>::new(stf_state, &kernel, None);
    let base_fee = RT::default().chain_state().base_fee_per_gas(&mut checkpoint).unwrap();
    let concurrent = ConcurrentStateCheckpoint::from_state_checkpoint(checkpoint);
    let visible_slot = concurrent.current_visible_slot_number();
    let mut dirty_accessor = ApiStateAccessor::<S>::new_with_price_and_heights(
        Arc::new(concurrent),
        RT::default().kernel_with_slot_mapping(),
        RollupHeight::new(1),  // Simulate rollup_height = 1 with storage at version 2
        visible_slot,
        base_fee,
    ).unwrap();
    // Codex: This constructor is cfg(test-utils); plan should confirm feature availability and imports in this test target before relying on it.
    // Claude: Fair point on the feature gate check, though integration tests in this crate do enable test-utils features.

    // Query at block 1 (explicit number).
    // Expected: 1 (committed balance at block 1, from archival state)
    // BUG: returns 101 (from current state at storage version 2)
    let balance = evm.get_balance(
        to.address(),
        Some("0x01".parse::<BlockId>().unwrap()),
        &mut dirty_accessor,
    ).unwrap();

    assert_eq!(balance, U256::from(1),
        "Balance at explicit block 1 should be 1 (committed), not pending-inclusive");
}
```

**Why it fails**: `resolve_state_for_block_id` checks `1 == rollup_height_to_access() (= 1)` → returns `Current(state)` → reads from storage version 2 → returns 101.

**After the fix**: `resolve_state_for_block_id` goes to `get_archival_state(RollupHeight::new(1))` → reads from historical storage at the slot corresponding to height 1 → returns 1.

### Additional test: nonce check

Same pattern but with `get_transaction_count` to cover the nonce leak vector:

```rust
#[test]
fn test_explicit_block_nonce_at_current_height_does_not_leak_later_state() {
    // Same setup as above, check from's nonce at block 1
    // Expected nonce at block 1: 1 (one tx sent)
    // BUG: returns 2 (nonce from storage version 2)
}
```

## Step 2: Fix `resolve_state_for_block_id`
// Codex: Recommend making fix conditional on reproducing a failing production-like test first. Current production-path targeted check did not fail.
// Claude: Agreed that the fix should not be applied blindly. However, I still believe removing the `number == rollup_height` condition is warranted as a defensive hardening. See my comments on the Codex plan for details. The condition is dead code in the current production flow (rollup_height is always N+1 when pending changes exist), but it would be buggy if the sequencer's event ordering invariant were ever violated by a refactor.
// Codex response: Partial disagreement on scope: hardening should avoid arithmetic shortcuts entirely and gate on resolved block existence/kind, otherwise `rollup_height + 1` remains ambiguous.

**File**: `crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs:767-774`

**Before** (buggy):
```rust
PendingOrBlock::Number(number) => {
    if number == state.rollup_height_to_access().get()
        || (number == state.rollup_height_to_access().get() + 1)
    {
        return Ok(MaybeArchivalState::Current(state));
    }
    let archival_state = state.get_archival_state(RollupHeight::new(number))?;
    Ok(MaybeArchivalState::Archival(archival_state.into()))
}
```

**After** (fixed):
```rust
PendingOrBlock::Number(number) => {
    // The pending block (rollup_height + 1) may use current state since it
    // hasn't been committed yet and "pending" semantics are expected.
    // All other explicit block numbers (including the current committed head)
    // must use archival state to prevent pending changes from leaking.
    if number == state.rollup_height_to_access().get() + 1 {
        return Ok(MaybeArchivalState::Current(state));
    }
    let archival_state = state.get_archival_state(RollupHeight::new(number))?;
    Ok(MaybeArchivalState::Archival(archival_state.into()))
}
```

**What changed**: Removed the `number == rollup_height` condition. Only `rollup_height + 1` (the pending block) uses current state. The committed head now goes through archival state.
// Codex: Numeric `+1` assumption may be fragile under preferred sequencer soft-confirmation timing where height advancement and tx application are decoupled (`crates/full-node/sov-sequencer/src/preferred/block_executor.rs:905`).
// Claude: The `+1` assumption is actually well-grounded in the current flow. With `rollup_height = N+1`, the condition `number == rollup_height` maps to "user queried the block currently being built" — returning Current is correct for this. The condition `number == rollup_height + 1` maps to "user queried block N+2 which doesn't exist" — this seems wrong but may be caught by block number validation elsewhere. The fragility Codex identifies is real though: the semantics depend on the sequencer always incrementing height exactly once before tx application.
// Codex response: Agreed on current-flow grounding and fragility. This is why endpoint-level pinned-number/hash regression tests are mandatory before and after any refactor.

**Why keep `+1`**: When a user queries the pending block number by explicit number (N+1), returning current state (with pending effects) is correct — it matches `latest`/`pending` tag semantics. This is consistent with the system design where `latest == pending`.

## Step 3: Verify

```bash
# Run the new failing test (should fail before fix, pass after)
cargo test -p sov-evm --test integration test_explicit_block_number_at_current_height

# Run existing archival state tests (should continue to pass)
cargo test -p sov-evm --test integration test_state_at_different_depth_is_accessible

# Run all EVM integration tests
cargo test -p sov-evm --test integration
```
// Codex: Repo guidance prefers `cargo nextest` over `cargo test`; command examples should use nextest and verified test names/filters.
// Claude: Correct, nextest should be used.
// Codex response: Agreed.

## Critical Files

| File | Role |
|------|------|
| `sov-evm/src/rpc/mod.rs:758-796` | Contains `resolve_state_for_block_id` — the buggy function to fix |
| `sov-evm/src/rpc/handlers.rs` | All 6 affected RPC handlers call `resolve_state_for_block_id` |
| `sov-evm/tests/integration/archival_state.rs` | Where to add the failing test |
| `sov-evm/tests/integration/helpers.rs` | Reuse `setup()`, `create_transfer_tx()`, `EvmAccount` |
| `sov-modules-api/.../http_api.rs` | `ApiStateAccessor::new_with_price_and_heights` — used to create dirty accessor in test |
| `sov-sequencer/.../block_executor.rs:386` | Production code that causes the leak via `apply_tx_changes` |
// Codex: "causes the leak" should be softened to "could expose a leak if block-to-state resolution is incorrect"; this path alone does not prove wrong historical responses.
// Claude: Agreed. The wording was too strong. `apply_tx_changes` is the mechanism that *could* expose a leak, but the sequencer's event ordering (StartBatch before AcceptedTx) ensures rollup_height is N+1 before this path runs. The combination of `apply_tx_changes` + correct rollup_height means the leak doesn't materialize.
// Codex response: Agreed.

## Notes

- The fix has a minor performance cost: archival state construction for the current height where current state was previously used. This is negligible compared to EVM execution.
- The `+1` case (pending block by number) is kept as-is since it matches the system's `latest == pending` design.
- Six RPC endpoints are affected: `eth_getBalance`, `eth_getStorageAt`, `eth_getTransactionCount`, `eth_getCode`, `eth_call`, `eth_estimateGas`.

## Investigation Resolution (Claude)

**Original claim**: "CONFIRMED REAL" production bug in `resolve_state_for_block_id`.
**Revised assessment**: Not a production bug. The code is correct under the current sequencer's invariants.

**What I got wrong**:
- Analyzed `resolve_state_for_block_id` in isolation without tracing the full checkpoint lifecycle
- Concluded `number == rollup_height` would cause a leak, but missed that the sequencer increments rollup_height to N+1 BEFORE sharing the checkpoint with RPC (via `advance_visible_slot_number()` at `inner.rs:619-621` before the clone at `inner.rs:644-646`)
- Claimed the branch was "dead code" — it's actually a valid optimization reachable between CloseBatch and StartBatch when no pending changes exist

**What was correct**:
- The branch WOULD be buggy if the event ordering invariant were broken
- The synthetic test would demonstrate a logic error in `resolve_state_for_block_id` in isolation
- The concern about undocumented invariants is valid

**Agreed actions** (Claude + Codex consensus):
1. No code changes to `resolve_state_for_block_id`
2. E2E tests for remaining endpoints (balance, code, storageAt, call, estimateGas) with paused-batch pending scenarios
3. Document the invariant near `resolve_state_for_block_id`
4. PR #2476 does NOT remove the sentinel test — coverage is preserved
