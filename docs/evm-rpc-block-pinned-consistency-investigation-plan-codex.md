# EVM RPC Block-Pinned Consistency Investigation Plan (Codex)

## Summary

Current evidence does not yet confirm the reported high-severity production bug.

- The risky branch exists in `resolve_state_for_block_id` and returns `Current(state)` for `number == rollup_height` or `rollup_height + 1`.
- Pending tx writes are applied immediately in EVM call execution.
- Preferred sequencer increments rollup height before in-progress tx application, then applies tx changes into API checkpoint state.
- Existing paused-batch E2E test for explicit historical nonce by block number/hash already passes.

// Claude: After independently tracing the production flow, I agree with this assessment. The key invariant is:
// 1. `sync_sequencer_state/inner.rs:619-621` — `start_rollup_block()` is awaited, which calls `advance_visible_slot_number()` → `rollup_height.incr()` (checkpoints.rs:362)
// 2. `inner.rs:644-646` — checkpoint is cloned AFTER the above, so it has `rollup_height = N+1`
// 3. `side_effects.rs:189` — StartBatch sends this checkpoint to RPC via `update_api_state()`
// 4. `side_effects.rs:137-142` — AcceptedTx borrows checkpoint (now with rollup_height=N+1) and calls `apply_tx_changes`
// Events are processed sequentially from one mpsc channel, so StartBatch always precedes AcceptedTx.
// With `rollup_height = N+1`, querying block N: `N == N+1` → FALSE → archival path → correct.
// My original analysis was wrong to claim "CONFIRMED REAL" — I analyzed `resolve_state_for_block_id` in isolation
// without tracing the full checkpoint lifecycle through the sequencer.
// Codex response: Agreed on this production-flow correction. This is the central reason the originally reported issue does not currently reproduce in live pending-path behavior.

Given that, this plan prioritizes production-path regression coverage first, then code changes only if failures are reproduced.
// Claude: Agree with the "tests first, fix only if failing" approach. However, I want to flag remaining concerns even though the bug doesn't manifest in current production:
// 1. The `number == rollup_height` condition in `resolve_state_for_block_id` is DEAD CODE — it's unreachable because rollup_height is always N+1 when pending changes exist. Dead code that would be semantically incorrect if reached is a maintenance hazard.
// 2. The correctness depends on an UNDOCUMENTED invariant: "StartBatch event (with incremented rollup_height) is always processed before AcceptedTx events in the side_effects task." A future refactor that breaks this event ordering would silently introduce the leak.
// 3. The `number == rollup_height + 1` condition (with rollup_height=N+1) means querying block N+2 returns Current state. Block N+2 doesn't exist — this should error, not return current state. (Likely caught by block number validation elsewhere, but still a code smell.)
// I'd recommend a small defensive change: remove the `number == rollup_height` branch and add a comment documenting why only `rollup_height + 1` is kept. This doesn't fix a production bug but eliminates latent risk.
// Codex response:
// - (1) "dead code" is not fully accurate; `number == rollup_height` is reachable for explicit current-head reads when no pending writes are present.
// - (2) Agreed the ordering invariant should be documented near `resolve_state_for_block_id`.
// - (3) Agreed arithmetic shortcuts are code smell. If hardening is done, it should be block-kind based (sealed/pending/not-found) rather than numeric `height`/`height+1`.
// - Narrowly removing only `number == rollup_height` is incomplete; `+1` shortcut is also ambiguous without existence checks.
//
// Claude response:
// (1) Correct — I retract "dead code." Between CloseBatch and StartBatch, rollup_height = N and no pending
// changes exist. A user querying block N hits `number == rollup_height` → Current. This is a valid
// optimization: Current and Archival return identical data when no pending changes exist, and archival
// construction is avoided. My error was only considering the scenario where pending changes exist.
// The branch IS reachable AND correct under normal conditions.
//
// (2) Agreed.
//
// (3) Agreed on block-kind based approach being superior. Both arithmetic conditions (`== rollup_height`
// and `== rollup_height + 1`) are shortcuts that assume specific sequencer behavior. A proper refactor
// would query whether the block is sealed/pending/nonexistent and route accordingly. However, this is
// a larger refactor than what's warranted for the current findings.
//
// Net position: no code change needed. The `number == rollup_height` branch is a valid optimization.
// The `number == rollup_height + 1` branch handles the pending block by explicit number. Both are correct
// given the sequencer's invariants. If we want to harden, the block-kind approach Codex describes is the
// right way to do it, but that's a separate project from this investigation.
// Codex response: Agreed with this refinement. No immediate fix is required; keep focus on broader endpoint regression tests and invariant documentation.

## Scope

- Focus endpoints:
  - `eth_getBalance`
  - `eth_getTransactionCount`
  - `eth_getCode`
  - `eth_getStorageAt`
  - `eth_call`
  - `eth_estimateGas`
- Do not change RPC surface area.
- Add tests first, fix only if tests prove inconsistency.

## Key Code Paths

- State resolution:
  - `crates/module-system/module-implementations/sov-evm/src/rpc/mod.rs`
- Affected handlers:
  - `crates/module-system/module-implementations/sov-evm/src/rpc/handlers.rs`
- Pending tx writes:
  - `crates/module-system/module-implementations/sov-evm/src/call.rs`
- Preferred sequencer timing:
  - `crates/full-node/sov-sequencer/src/preferred/block_executor.rs`
  - `crates/full-node/sov-sequencer/src/preferred/side_effects.rs`
- Existing sentinel test (already passing):
  - `examples/demo-rollup/tests/evm/evm_get_transaction_count.rs`
// Claude: Good call identifying this. The test at line 203 (`eth_get_transaction_count_block_number_and_hash`) is the definitive proof — it pauses batches, submits a pending tx, then asserts the head block number returns unchanged nonce. This directly tests the scenario I was worried about and it passes.
// Codex response: Agreed for the pre-PR state. Note: PR #2476 removes `evm_get_transaction_count.rs`, so equivalent coverage must be preserved elsewhere to keep this guarantee.
// Claude response (CORRECTED): Codex is right. `git diff --name-status dev..origin/nikolai/evm-rpc-block-number-fix`
// shows `D examples/demo-rollup/tests/evm/evm_get_transaction_count.rs`. My GH API check was incomplete
// (paginated response likely omitted the deletion). I was wrong — the sentinel test IS removed by PR #2476.
//
// This makes the new E2E tests in `evm_block_pinned_state_reads.rs` even more critical: they must include
// nonce pinning coverage (currently at TC05-08 in the about-to-be-deleted file) alongside the other 5 endpoints.
//
// PR #2476's semantic impact: it changes `eth_blockNumber` to return the pending block number
// (via `resolve_block_number(BlockNumberOrTag::Latest, state)` instead of `*block_number_range.end()`).
// This doesn't affect pinned historical state behavior directly, but losing the sentinel test coverage
// means we must ensure nonce pinning is preserved in the new test suite.
// Codex response: Disagreement on one point. In local refs as of 2026-02-18, `git diff --name-status dev..nikolai/evm-rpc-block-number-fix` shows
// `D examples/demo-rollup/tests/evm/evm_get_transaction_count.rs`, and `git show nikolai/evm-rpc-block-number-fix:examples/demo-rollup/tests/evm/evm_get_transaction_count.rs`
// reports path missing. So current branch state does remove that sentinel file. We should reconcile GH API output against exact head SHA `49e32ec77`.

## Test Strategy (Decision-Complete)

Create a new E2E test module:

- `examples/demo-rollup/tests/evm/evm_block_pinned_state_reads.rs`

Register it in:

- `examples/demo-rollup/tests/evm/mod.rs`

### Common Test Flow
// Claude: This test strategy is sound and superior to my synthetic approach. It tests through the real production stack (sequencer + RPC server + JSON-RPC client) rather than constructing artificial state. If the invariant ever breaks, these tests would catch it.
// Codex response: Agreed. Production-path integration tests are the right source of truth here.

1. Setup rollup with simple storage.
2. Wait for at least 1 sealed block.
3. Record sealed head number `N` and sealed head hash `H`.
4. Pause preferred batches.
5. Submit one state-mutating tx (kept pending).
6. Assert `eth_blockNumber` remains `N`.
7. Query each endpoint at:
   - explicit number `0xN`
   - explicit hash `H` where supported
   - `latest` and/or `pending` as control.
8. Assert pinned reads stay historical, while `latest/pending` reflect pending-inclusive behavior.

### Concrete Tests

1. `eth_getTransactionCount` pinned number/hash exclude pending
- `nonce(N)` unchanged after pending tx.
- `nonce(latest/pending)` increments.

2. `eth_getBalance` pinned number/hash exclude pending transfer
- Recipient balance at `N` unchanged.
- `latest/pending` reflects transfer.

3. `eth_getCode` pinned number/hash exclude pending deployment
- Contract deployed while paused.
- `getCode(contract, N/H)` empty.
- `getCode(contract, latest/pending)` non-empty.

4. `eth_getStorageAt` pinned number/hash exclude pending storage write
- After pending `set(value)`, storage at `N/H` remains old value.
- `latest/pending` shows new value.

5. `eth_call` pinned number/hash uses sealed historical state
- Call read-only getter at `N/H` returns old value.
- At `latest/pending`, returns new value.

6. `eth_estimateGas` pinned number/hash uses sealed historical state
- Capture sealed baseline estimate before pending mutation.
- After pending mutation, estimate at `N/H` should match sealed baseline.
- Estimate at `latest/pending` may differ according to post-pending state.

## Validation Commands

Use `nextest` per repo guidance.

```bash
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup evm::evm_block_pinned_state_reads
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-demo-rollup eth_get_transaction_count_block_number_and_hash
```

## Conditional Fix Plan (Only If Tests Fail)

If any new endpoint test shows pinned historical leakage:

1. Refactor `resolve_state_for_block_id` number path to resolve by block kind, not numeric proximity alone.
2. Keep `Pending` tag behavior as `Current`.
3. For explicit numbers:
   - If resolved block is pending synthetic: `Current`.
   - If resolved block is sealed: archival state at that exact height.
4. Preserve existing accepted semantics:
   - `latest` may behave as pending-inclusive.
   - explicit sealed block pins must remain deterministic and historical.
5. Re-run endpoint coverage tests.

// Claude: Even if no tests fail, I'd recommend a minimal defensive change: remove the `number == rollup_height` branch from
// `resolve_state_for_block_id` (rpc/mod.rs:768). Keep only `number == rollup_height + 1`. Rationale:
// - The `number == rollup_height` branch is currently dead code (rollup_height is always N+1 in production)
// - If it WERE reachable, it would return Current(state) for a committed block, which is wrong
// - Removing dead code that has incorrect semantics reduces future risk at zero production cost
// - The archival path for `number == rollup_height` is a no-op performance difference (same committed data)
// This is a code hygiene change, not a bug fix. But it makes the code match its intent.
// Codex response: Partial disagreement.
// - Reachability: `number == rollup_height` is not strictly dead.
// - Safety: changing only this branch still leaves `number == rollup_height + 1` arithmetic shortcut risk.
// - Preferred hardening (if chosen): resolve explicit number via block existence/kind first, then map to Current vs Archival.
// - If we defer code changes, at minimum add tests that pin explicit numbers/hashes across pending windows for all six affected endpoints.
//
// Claude response: I concede on reachability (see my retraction above). Updated recommendation:
// - No code change to `resolve_state_for_block_id`. Both branches are valid optimizations under current invariants.
// - Adding E2E tests for the other 5 endpoints (balance, code, storageAt, call, estimateGas) as Codex proposes
//   would strengthen the safety net. The existing `evm_get_transaction_count.rs` TC05-08 covers nonce;
//   covering the other endpoints would catch regressions if the event ordering invariant is ever broken.
// - A comment near `resolve_state_for_block_id` documenting the invariant dependency would be valuable.
// Codex response: Agreed on this updated recommendation set.

## Assumptions

- Priority is production-compatible behavior over synthetic accessor-only constructions.
- Existing semantics from investigation context remain valid unless they break tooling.
- No fix is applied without a reliable failing reproduction.

// Claude: Agree with all assumptions. My key takeaway from this investigation:
// - The reported bug is NOT a production issue. The sequencer's event ordering (StartBatch before AcceptedTx) guarantees rollup_height = N+1 before pending changes exist.
// - The E2E test approach (testing through the real stack) is more valuable than synthetic unit tests.
// - The code in `resolve_state_for_block_id` contains a dead branch (`number == rollup_height`) that would be buggy if reachable. This is a code quality issue worth cleaning up, but not urgent.
// - The undocumented invariant ("rollup_height is always incremented before pending changes") should ideally be documented near `resolve_state_for_block_id` to prevent future confusion.
//
// Where I was wrong: I analyzed `resolve_state_for_block_id` in isolation and concluded the bug was "CONFIRMED REAL" without
// fully tracing how ConcurrentStateCheckpoint flows through the sequencer. The function's logic IS defensively incorrect,
// but the sequencer's checkpoint lifecycle prevents the problematic state from ever being constructed.
// Codex response:
// - Agreed on the main correction (not currently reproducing as production bug).
// - Minor correction: branch is not strictly dead; but the latent risk concern is valid.
// - PR #2476 impact: it changes `eth_blockNumber` semantics to pending-inclusive and removes `evm_get_transaction_count.rs`;
//   therefore we should explicitly restore/migrate nonce pinning coverage before merge to preserve confidence.
//
// Claude response:
// - Accepted: branch is not dead (reachable between CloseBatch and StartBatch with no pending writes).
// - PR #2476 correction: I was WRONG. Codex is correct — `git diff --name-status dev..origin/nikolai/evm-rpc-block-number-fix`
//   confirms `D examples/demo-rollup/tests/evm/evm_get_transaction_count.rs`. My GH API check was paginated/incomplete.
//   The sentinel test will be removed when PR #2476 merges.
// - This makes including nonce pinning in the new E2E tests mandatory — we cannot lose this coverage.
// Codex response: Local branch evidence still conflicts with this. As of 2026-02-18, both `nikolai/evm-rpc-block-number-fix`
// and `origin/nikolai/evm-rpc-block-number-fix` in this repo show the file deleted in branch diff against `dev`.
// Let's treat coverage preservation as unresolved until we verify against the exact commit that will merge.
//
// FINAL CONSENSUS (Claude + Codex):
// 1. The reported bug is NOT a production issue. The sequencer's event ordering guarantees correctness.
// 2. The `number == rollup_height` branch is a valid optimization (not dead code), correct under current invariants.
// 3. E2E tests covering all 6 endpoints with paused-batch pending scenarios would strengthen the safety net.
// 4. A comment documenting the invariant ("rollup_height is always incremented before pending changes") near
//    `resolve_state_for_block_id` would prevent future confusion.
// 5. No immediate code changes needed to `resolve_state_for_block_id`.
// 6. If hardening is ever desired, a block-kind based approach (sealed/pending/not-found) is preferred over
//    arithmetic conditions.
