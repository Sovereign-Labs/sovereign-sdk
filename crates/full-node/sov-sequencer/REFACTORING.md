# PreferredSequencer Refactoring Plan

**Status:** Planning
**Owner:** @leo
**Started:** 2025-12-16
**Target Completion:** 2025-01-13 (4 weeks)
**Last Updated:** 2025-12-16

---

## Executive Summary

The PreferredSequencer module (1,498 LOC in mod.rs) is **well-architected** but suffers from organizational debt. The goal is to improve maintainability through incremental, low-risk refactoring that:

1. Extracts initialization into builder pattern (~250 LOC)
2. Separates recovery logic into dedicated module (~150 LOC)
3. Moves pure functions to focused modules (~250 LOC)
4. Adds 200+ unit tests (currently only 34)
5. Reduces E2E test dependence (currently ~6,819 LOC of integration tests)

**Key Principle:** Incremental, behavior-preserving changes with comprehensive testing.

---

## Current State Analysis

### File Structure
```
preferred/mod.rs (1,498 LOC)
├── Initialization (lines 160-413) - 254 LOC
├── Recovery Logic (lines 441-537) - 97 LOC
├── Sync/Wait Logic (lines 557-614) - 58 LOC
├── Transaction Handling (lines 617-768) - 152 LOC
├── State Update Orchestration (lines 801-942) - 142 LOC
├── Sequencer Trait Implementation (lines 944-1105) - 162 LOC
└── Helper Functions (remainder) - ~633 LOC
```

### Metrics
- **LOC:** 1,498 (mod.rs alone)
- **Unit tests:** 34 (mostly in mod.rs)
- **Integration tests:** ~6,819 LOC
- **TODOs/FIXMEs:** 19 across codebase
- **unwrap/expect:** 447 calls
- **panic!:** 45 calls
- **clone():** 296 operations

### Strengths
- ✅ Excellent use of channels (clean actor pattern)
- ✅ Proper shutdown handling (watch::Receiver pattern)
- ✅ Good separation of concerns (DB, executor, blob sender separate)
- ✅ Well-designed error types (`PreferredSeqOperation` enum)
- ✅ Comprehensive metrics instrumentation
- ✅ Graceful recovery strategy

### Opportunities for Improvement
- 📦 Initialization could be more modular (builder pattern)
- 🔄 Recovery logic is self-contained (could be separate module)
- 🧩 Helper functions could be in focused modules
- 📝 Code navigation would benefit from smaller files
- 🧪 More unit tests would speed up development

---

## Phase 1: Non-Breaking Extractions (Week 1-2)

**Goal:** Improve organization without changing behavior. Zero risk.

### ✅ Step 1.1: Extract Pure Functions to `slot_calculations.rs`
**Status:** Not Started
**Estimated Time:** Day 1
**Risk:** Low

**Files to create:**
- `preferred/slot_calculations.rs` (~250 LOC)

**Functions to move:**
- `slot_count_delta_acceptable_lower_bound()`
- `raw_max_deferred_slots_delay()`
- `next_visible_slot_number_increase()`
- `next_visible_slot_number_increase_inner()`
- `is_lagging_less_than_ideal_amount()`
- `get_next_sequence_number_according_to_node()`
- `current_visible_slot_number_according_to_node()`
- `accepts_preferred_batches()`
- All tests from `mod.rs` (lines 1348-1498)

**Benefits:**
- Pure functions easier to test
- Clear naming shows these are calculations
- Tests live next to code
- Reduces mod.rs by ~200 LOC

**Testing:**
- [ ] Move existing tests
- [ ] Add 15 new unit tests
- [ ] Add property-based tests
- [ ] Target: 100% coverage

---

### ✅ Step 1.2: Extract Error Constructors to `error_responses.rs`
**Status:** Not Started
**Estimated Time:** Day 1
**Risk:** Low

**Files to create:**
- `preferred/error_responses.rs` (~60 LOC)

**Functions to move:**
- `rate_limit_error()`
- `replica_mode_error()`
- `shut_down_error()`
- `err_cant_fit_tx()`

**Benefits:**
- Error construction centralized
- Easy to ensure consistency
- Reduces mod.rs by ~50 LOC

**Testing:**
- [ ] Add 10 unit tests for error structure
- [ ] Verify HTTP status codes
- [ ] Test JSON details formatting

---

### ✅ Step 1.3: Extract Initialization to Builder Pattern
**Status:** ✅ Completed (2025-12-16)
**Estimated Time:** Days 2-3
**Risk:** Low

**Files to create:**
- `preferred/initialization.rs` (~300 LOC)

**Structure:**
```rust
pub struct PreferredSequencerBuilder<S, Rt, Da> {
    da: Da,
    config: SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>,
}

impl<S, Rt, Da> PreferredSequencerBuilder<S, Rt, Da> {
    pub fn new(da: Da, config: SequencerConfig<...>) -> Self;

    pub async fn build(self, ...) -> anyhow::Result<(PreferredSequencer<S, Rt, Da>, Vec<JoinHandle<()>>)> {
        // Break into smaller methods:
        self.setup_oracle_config()?;
        self.setup_database().await?;
        self.setup_blob_sender().await?;
        self.setup_state_root_compute().await?;
        self.setup_sync_state().await?;
        self.spawn_background_tasks().await
    }
}
```

**Changes to mod.rs:**
```rust
impl<S, Rt, Da> PreferredSequencer<S, Rt, Da> {
    pub async fn create(...) -> anyhow::Result<(Self, Vec<JoinHandle<()>>)> {
        PreferredSequencerBuilder::new(da, config)
            .build(...)
            .await
    }
}
```

**Benefits:**
- Initialization logic testable in isolation
- Each setup step is named and discoverable
- Reduces mod.rs by ~250 LOC
- Existing API unchanged

**Testing:**
- [ ] Add unit tests for each setup step
- [ ] Mock dependencies for testing
- [ ] Integration test that builder works end-to-end

---

### ✅ Step 1.4: Extract Recovery to `recovery.rs`
**Status:** Not Started
**Estimated Time:** Day 4
**Risk:** Low

**Files to create:**
- `preferred/recovery.rs` (~150 LOC)

**Structure:**
```rust
pub(super) struct RecoveryCoordinator<S, Rt> {
    config: Arc<SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>>,
    state_updator: Arc<SequencerStateUpdator<S, Rt>>,
}

impl<S, Rt> RecoveryCoordinator<S, Rt> {
    pub async fn recover_and_catch_up(...) -> anyhow::Result<()>;
    fn catchup_batches_to_send(...) -> (u64, u64);
    fn slot_count_delta_range(...) -> (u64, u64);
}
```

**Benefits:**
- Recovery logic independently testable
- Clear entry point for recovery scenarios
- Reduces mod.rs by ~100 LOC

**Testing:**
- [ ] Unit tests for catchup calculation
- [ ] Unit tests for slot delta range
- [ ] Mock async recovery flow
- [ ] Add 20 unit tests total

---

### ✅ Step 1.5: Extract Sync Helpers to `sync_helpers.rs`
**Status:** Not Started
**Estimated Time:** Day 5
**Risk:** Low

**Files to create:**
- `preferred/sync_helpers.rs` (~80 LOC)

**Structure:**
```rust
pub(super) struct NodeSyncHelper;

impl NodeSyncHelper {
    pub async fn wait_for_node_resync<S, Rt>(
        state_updator: &SequencerStateUpdator<S, Rt>,
        state_update_receiver: &mut StateUpdateReceiver<S::Storage>,
        shutdown_receiver: &watch::Receiver<()>,
        distance_to_tip: u64,
        current_info: StateUpdateInfo<S::Storage>,
    ) -> anyhow::Result<()>
}
```

**Benefits:**
- Eliminates duplication (3 similar methods → 1)
- More testable
- Reduces mod.rs by ~60 LOC

---

### Phase 1 Results (After Week 2)

**Before:**
- `preferred/mod.rs`: 1,498 LOC

**After:**
- `preferred/mod.rs`: ~800 LOC (↓ 47%)
- `preferred/initialization.rs`: ~300 LOC (new)
- `preferred/recovery.rs`: ~150 LOC (new)
- `preferred/sync_helpers.rs`: ~80 LOC (new)
- `preferred/slot_calculations.rs`: ~250 LOC (new)
- `preferred/error_responses.rs`: ~60 LOC (new)

**Total LOC:** Same (~1,640 LOC including new files)
**Readability:** Much improved
**Risk:** Zero - all behavior preserved
**Test Coverage:** Improved - 70+ new unit tests

---

## Phase 2: Complexity Reduction (Week 3)

**Goal:** Make core sequencer logic easier to follow.

### ✅ Step 2.1: Simplify `accept_tx_inner`
**Status:** Not Started
**Estimated Time:** Days 1-2
**Risk:** Low

**Current issues:**
- 152 lines
- 4 levels of nesting
- Complex error mapping

**Approach:**
```rust
async fn accept_tx_inner(...) -> Result<AcceptedTx<...>, ErrorObject> {
    self.validate_not_shutting_down()?;
    let tx_metadata = self.authenticate_and_extract_metadata(baked_tx, ip_addr).await?;
    self.apply_transaction_delay(tx_metadata.delay_ms, tx_metadata.tx_hash).await;
    let result = self.route_transaction_by_uniqueness(baked_tx, tx_metadata, original_tx_queue_id).await?;
    self.handle_transaction_result(result, tx_metadata).await
}
```

**Benefits:**
- Each step named and obvious
- Easier to add logging/metrics
- Simpler to modify any step
- Better stack traces

---

### ✅ Step 2.2: Add Documentation
**Status:** Not Started
**Estimated Time:** Day 3
**Risk:** None

**Targets:**
- Module-level docs with architecture diagram
- State machine documentation
- Concurrency model explanation
- Function-level docs for public APIs

---

### ✅ Step 2.3: Add Tracing Spans
**Status:** Not Started
**Estimated Time:** Day 3
**Risk:** Low

**Add to:**
- `accept_tx_inner` - transaction lifecycle
- `recover_and_catch_up` - recovery progress
- `replay_soft_confirmations_on_top_of_node_state` - replay progress

---

## Phase 3: Unit Testing (Week 4)

**Goal:** Add 200+ unit tests, reduce E2E dependence.

### Testing Pyramid Goal

```
         ▲
        / \          Unit Tests (200+)
       /   \         Fast, focused, deterministic
      /     \
     /       \
    /_________\
   /           \
  /             \    Integration Tests (50)
 /               \   Sequencer + component
/                 \
/___________________\
                     E2E Tests (20)
                     Full rollup scenarios
```

### ✅ Priority 1: Pure Functions (Days 1-2)
**Target:** 35 unit tests
**Coverage:** 100%
**Files:** `slot_calculations.rs`, `error_responses.rs`

**Tests to add:**
- [ ] Property tests for slot calculations
- [ ] Edge case tests (overflow, underflow)
- [ ] Error response structure tests
- [ ] JSON serialization tests

---

### ✅ Priority 2: State Machine Logic (Days 2-3)
**Target:** 30 unit tests
**Coverage:** 90%
**Files:** `sync_sequencer_state/conditions_table.rs`

**Tests to add:**
- [ ] `operation_for_master` decision logic
- [ ] `operation_for_replica` decision logic
- [ ] All condition combinations
- [ ] Property tests for determinism

---

### ✅ Priority 3: Batch Size Tracker (Day 3)
**Target:** 15 unit tests
**Coverage:** 100%
**Files:** `batch_size_tracker.rs`

**Tests to add:**
- [ ] `can_fit_tx` logic
- [ ] Execution time tracking
- [ ] Serialization overhead calculation
- [ ] Reset behavior

---

### ✅ Priority 4: Rate Limiter (Day 4)
**Target:** 25 unit tests
**Coverage:** 95%
**Files:** `rate_limiter/limiter.rs`

**Tests to add:**
- [ ] Token bucket refill logic
- [ ] User isolation
- [ ] Overflow protection
- [ ] Property tests for resource math

---

### ✅ Priority 5: Recovery Calculations (Day 5)
**Target:** 20 unit tests
**Coverage:** 85%
**Files:** `recovery.rs`

**Tests to add:**
- [ ] Catchup batch calculation
- [ ] Slot delta range calculation
- [ ] Edge cases (not lagging, very behind)
- [ ] Mock recovery flow

---

### Test Infrastructure

**Create:** `preferred/test_helpers.rs` (only in `#[cfg(test)]`)

```rust
#![cfg(test)]

/// Builder for test StateUpdateInfo
pub struct TestStateUpdateInfoBuilder<S: Spec> { ... }

/// Mock SequencerStateUpdator for testing
pub fn mock_state_updator<S: Spec, Rt: Runtime<S>>() -> (MockStateUpdator<S, Rt>, Receiver<Message<S, Rt>>);

/// Mock config builders
pub fn test_sequencer_config() -> PreferredSequencerConfig<TestAddress>;
```

---

## Metrics & Success Criteria

### Quantitative Goals
- ✅ mod.rs LOC: 1,498 → ~800 (47% reduction)
- ✅ Average function length: 40 → 20 LOC
- ✅ Max nesting depth: 4 → 2 levels
- ✅ Unit tests: 34 → 200+ (488% increase)
- ✅ Test execution time: 5 min → 30 sec for unit tests
- ✅ Code coverage: ~60% → 85%
- ✅ Zero behavior changes (proven by existing tests)

### Qualitative Goals
- ✅ New contributors find code faster
- ✅ "Where does recovery happen?" → Clear answer
- ✅ Code reviews faster (smaller diffs)
- ✅ Can run most tests without Docker/Postgres
- ✅ CI feedback in <2 minutes instead of 10

---

## Timeline

| Week | Focus | Deliverable | Risk |
|------|-------|-------------|------|
| 1 | Pure functions + Errors | 2 modules, -250 LOC, +25 tests | Low |
| 2 | Initialization + Recovery + Sync | 3 modules, -410 LOC, +50 tests | Low |
| 3 | Simplify accept_tx + Docs | Cleaner core logic | Medium |
| 4 | Unit testing push | +125 tests, test helpers | Low |

**Total:** 4 weeks, low-to-medium risk, high value

---

## Risk Mitigation

### Risk 1: Breaking Changes
**Mitigation:**
- Every step keeps public API identical
- Comprehensive integration tests after each change
- Use `#[deprecated]` for interim API changes

### Risk 2: Merge Conflicts
**Mitigation:**
- Do extractions before active feature work
- Coordinate with team on Slack
- Small PRs (one extraction per PR)

### Risk 3: Performance Regression
**Mitigation:**
- Profile before/after with benchmarks
- All code is `#[inline]` eligible
- Monitor metrics in staging

### Risk 4: Team Buy-in
**Mitigation:**
- Share this plan for feedback
- Pair with original author on first PR
- Emphasize benefits, not problems

---

## Migration Checklist (Per Extraction)

- [ ] Create new file
- [ ] Move code with zero changes
- [ ] Add `pub(super)` visibility
- [ ] Update imports in mod.rs
- [ ] Run: `env SKIP_GUEST_BUILD=1 cargo test -p sov-sequencer`
- [ ] Run: `env SKIP_GUEST_BUILD=1 cargo clippy -p sov-sequencer`
- [ ] Verify no behavior change
- [ ] Add unit tests for extracted code
- [ ] Update documentation
- [ ] Git commit with descriptive message
- [ ] Create PR with before/after comparison

---

## PR Template

```markdown
## Extract [Component Name] to dedicated module

This PR extracts [X LOC] from `preferred/mod.rs` into `preferred/[new_file].rs`
to improve code organization.

### Why
- Easier to understand [specific aspect]
- Each function can be tested independently
- Future changes won't bloat mod.rs

### What Changed
- New file: `preferred/[new_file].rs`
- Moved [list of functions] with zero behavior changes
- Added [N] unit tests

### Testing
- All existing integration tests pass
- Added unit tests with [coverage]% coverage
- Verified with: `env SKIP_GUEST_BUILD=1 cargo test -p sov-sequencer`

### Migration Guide
None - public API unchanged

### Before/After
- mod.rs: 1,498 → [new_count] LOC
- New module: [N] LOC
- New tests: [N]
```

---

## Notes & Observations

### 2025-12-16: Initial Analysis
- Identified `BatchCreationError` enum (line 1326) - well-designed error type
- Recovery logic is self-contained and extractable
- Pure functions are ideal first targets
- Team relationship with original author is good - opportunity to collaborate

### 2025-12-16: Initialization Builder Pattern Completed
- Created `preferred/initialization.rs` (300 LOC)
- `PreferredSequencer::create()` now delegates to `PreferredSequencerBuilder`
- Public API unchanged - backward compatible
- mod.rs reduced from 1,498 → 1,245 LOC (253 LOC reduction, 17%)
- Compilation successful with warnings only
- Next: Add unit tests for builder, then extract pure functions

---

## Open Questions

1. Should we extract `BatchCreationError` and related error types to a separate `errors.rs`?
2. Is there a preferred testing framework (proptest vs quickcheck)?
3. Should we coordinate with any ongoing feature work?
4. What's the team's preference for PR size (one extraction per PR vs multiple)?

---

## References

- PreferredSequencer: `crates/full-node/sov-sequencer/src/preferred/mod.rs`
- Integration tests: `crates/full-node/sov-sequencer/tests/integration/`
- Metrics: `crates/full-node/sov-sequencer/src/metrics.rs`

---

## Progress Tracking

### Week 1
- [ ] Day 1: Extract slot_calculations.rs + error_responses.rs
- [ ] Day 2-3: Extract initialization.rs (builder pattern)
- [ ] Day 4: Extract recovery.rs
- [ ] Day 5: Extract sync_helpers.rs

### Week 2
- [ ] Finalize extractions
- [ ] Add 75+ unit tests for extracted modules
- [ ] Review and merge PRs

### Week 3
- [ ] Simplify accept_tx_inner
- [ ] Add comprehensive documentation
- [ ] Add tracing spans

### Week 4
- [ ] Unit testing push
- [ ] Create test helpers
- [ ] Document testing patterns
- [ ] Final review and retrospective
