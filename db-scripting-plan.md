# DB Scripting Plan (Offline NOMT, Native-Only Raw Access)

## Scope and Constraints

- Execution mode: offline scripts against the on-disk DB.
- Backend scope: NOMT only.
- Language: Rust only.
- API shape: add new `_raw` methods (do not replace existing typed methods).
- Feature gating: all new scripting/raw APIs behind `native`.
- Value format: raw values are exposed as `Vec<u8>`.
- `StateVec` requirement: keep length decoded as `u64`; only element payloads should be raw-migratable.
- Container coverage goal: `StateMap`, `StateVec`, `StateValue` (including user/kernel/accessory namespaces unless complexity is disproportionate).

## Current Behavior and Interaction (What We Are Building On)

1. `NativeStorage::maybe_iter_user_values_with_prefix` exists and is optional [`crates/module-system/sov-state/src/storage.rs:402`].
2. That iterator is currently only consumed by pinned cache warmup (`PinnedCache::try_load_bucket_if_absent`) [`crates/module-system/sov-state/src/pinned_cache.rs:203`].
3. JMT `ProverStorage` returns `None` for this iterator (no prefix iteration support) [`crates/module-system/sov-state/src/prover_storage.rs:545`].
4. NOMT `NomtProverStorage` implements user-prefix iteration via historical state and filters tombstones [`crates/module-system/sov-state/src/nomt/prover_storage.rs:727`].
5. `StateMap`, `StateVec`, and `StateValue` currently decode values on read (`get_decoded`) and therefore cannot read incompatible bytes without failing decode:
   - `StateMap::get` [`crates/module-system/sov-modules-api/src/containers/map.rs:223`]
   - `StateVec::get` via map [`crates/module-system/sov-modules-api/src/containers/vec.rs:140`]
   - `StateValue::get` [`crates/module-system/sov-modules-api/src/containers/value.rs:105`]
6. Raw reads already exist at lower level via `StateReader::get(&SlotKey) -> Option<SlotValue>` [`crates/module-system/sov-modules-api/src/state/traits.rs:286`].
7. `StateVec` layout is already split into decoded length (`StateValue<u64>`) + element map (`StateMap<u64, V>`) [`crates/module-system/sov-modules-api/src/containers/vec.rs:37`, `crates/module-system/sov-modules-api/src/containers/vec.rs:38`].

## Target Script UX

Scripts should be able to:

1. Keep typed keys (including map keys and vec indices).
2. Read/write/remove values as raw `Vec<u8>`.
3. Iterate containers for migrations (especially map-like data) without forcing decode of old value type.
4. Re-encode and write new value format in place.
5. Reuse existing container prefixes/codecs so key derivation stays consistent with runtime.

## Proposed API Additions (Native-Only)

## `NamespacedStateValue`

- `get_raw(&self, state: &mut impl StateReader<N>) -> Result<Option<Vec<u8>>, _>`
- `set_raw(&mut self, value: &[u8], state: &mut impl StateWriter<N>) -> Result<(), _>`
- `remove_raw(&mut self, state: &mut impl StateReaderAndWriter<N>) -> Result<Option<Vec<u8>>, _>`

Implementation approach:

- Reuse existing `slot_key()` [`crates/module-system/sov-modules-api/src/containers/value.rs:78`].
- Use `StateReader::get` / `StateWriter::set` with `SlotValue::from(Vec<u8>)`.
- No value decode.

## `NamespacedStateMap`

- `get_raw<Kq>(&self, key: &Kq, state: &mut impl StateReader<N>) -> Result<Option<Vec<u8>>, _>`
- `set_raw<Kq>(&mut self, key: &Kq, value: &[u8], state: &mut impl StateWriter<N>) -> Result<(), _>`
- `remove_raw<Kq>(&self, key: &Kq, state: &mut impl StateReaderAndWriter<N>) -> Result<Option<Vec<u8>>, _>`

Iteration helpers:

- `iter_raw(...)` for prefix iteration where backend supports it (see storage extensions below).
- `iter_raw_from_keys(keys, state)` fallback helper for all namespaces/backends:
  - input: iterator of typed keys
  - output: typed key + optional raw bytes
  - useful for accessory and for cases where prefix iteration is unavailable.

Key decode for prefix iterators:

- Decode typed key from `slot_key.without_prefix()` using `codec.key_codec().try_decode(...)`.
- Return structured error on key decode failure (do not panic).

## `NamespacedStateVec`

- `get_raw(index, state) -> Result<Option<Vec<u8>>, _>`
- `set_raw(index, value, state) -> Result<Result<(), StateVecError<N>>, _>`
- `push_raw(value, state) -> Result<(), _>`
- `pop_raw(state) -> Result<Option<Vec<u8>>, _>`
- `iter_raw(state) -> Result<StateVecRawIter<...>, _>`

Details:

- Keep `len()` behavior unchanged and decoded via existing `StateValue<u64>` path [`crates/module-system/sov-modules-api/src/containers/vec.rs:167`].
- Raw element operations delegate to `elems` raw map methods.

## Storage-Layer Work Needed for Iteration

## NativeStorage trait extensions

Add namespace-aware prefix iterators (native-only):

- `maybe_iter_kernel_values_with_prefix(prefix: SlotKey) -> anyhow::Result<Option<impl Iterator<Item = (SlotKey, SlotValue)>>>`
- Keep existing `maybe_iter_user_values_with_prefix(...)`.
- Accessory prefix iterator is deferred (see below).

Why:

- NOMT already has kernel prefix iteration in `HistoricalStateReader` [`crates/full-node/sov-db/src/historical_state.rs:155`].
- Current public storage trait only exposes user-prefix iteration [`crates/module-system/sov-state/src/storage.rs:402`].

## NOMT implementation

- Implement `maybe_iter_kernel_values_with_prefix` in `NomtProverStorage` by using `historical_state.iter_kernel_values_with_prefix(...)` and filtering tombstones like user iteration.

## Accessory iteration note (significant complexity)

- `AccessoryDb` currently exposes point lookups (`get_value_option`) but no prefix iterator [`crates/full-node/sov-db/src/accessory_db.rs:44`].
- Efficient prefix iteration would require new indexing/schema work across versioned accessory state.
- Plan decision: in V1, support accessory raw point reads/writes/removes and `iter_raw_from_keys` fallback; defer accessory prefix scan to follow-up.

This still satisfies the “all containers” objective while avoiding disproportionate storage-index work in the first pass.

## Script Workflow (Planned Happy Path)

1. Open NOMT storage/checkpoint in native mode at target DB path/version.
2. Instantiate same containers (same prefixes/codecs/types) used by runtime.
3. Use `_raw` APIs:
   - `StateMap`: iterate raw entries (user/kernel via prefix iterator; accessory via key list fallback).
   - `StateVec`: iterate indices using decoded `len`, read raw element bytes.
   - `StateValue`: read raw singleton bytes.
4. Convert old bytes to new bytes in script logic.
5. Write back with `_raw` setters.
6. Commit state changes through existing storage commit flow.

## Testing Plan

1. Unit tests in container modules:
   - `_raw` read/write/remove for map/value/vec.
   - vec raw ops preserve length semantics.
   - key decode failures in map raw iter are surfaced as errors.
2. NOMT integration tests:
   - migration from incompatible value schema for `StateMap`.
   - migration for `StateVec` elements while preserving decoded length.
   - migration for `StateValue`.
   - kernel prefix iteration parity with user behavior.
3. Regression checks:
   - existing typed APIs still behave unchanged.
   - feature gate coverage (`native` only).
   - JMT behavior remains `None` for optional iterators (no behavior change expected).

## Rollout Sequence

1. Add raw container methods (`StateValue`, `StateMap`, `StateVec`) behind `native`.
2. Add kernel prefix iterator support to `NativeStorage` + NOMT implementation.
3. Add map raw iteration helper using storage prefix iterators.
4. Add fallback key-driven raw iteration helper for all namespaces.
5. Add docs/examples for offline migration scripts.
6. Optional follow-up: accessory prefix index + true accessory prefix iteration.

## PR-Sized Implementation Sequence

## PR1: Raw Singleton APIs (`StateValue`)

Scope:

- Add `#[cfg(feature = "native")]` raw methods on `NamespacedStateValue`:
  - `get_raw`
  - `set_raw`
  - `remove_raw`
- Keep existing typed methods unchanged.

Primary files:

- `crates/module-system/sov-modules-api/src/containers/value.rs`

Tests:

- Add/extend unit tests in value container tests (native) to verify:
  - raw roundtrip
  - raw remove semantics
  - compatibility with existing typed methods in same object lifecycle

Out of scope:

- Map/vec raw methods.
- Any storage trait changes.

## PR2: Raw Map Point APIs (`StateMap`)

Scope:

- Add `#[cfg(feature = "native")]` raw point methods on `NamespacedStateMap`:
  - `get_raw`
  - `set_raw`
  - `remove_raw`
- Preserve typed key encoding path (`slot_key`).

Primary files:

- `crates/module-system/sov-modules-api/src/containers/map.rs`

Tests:

- Add/extend map tests for:
  - typed key + raw value write/read/remove
  - coexistence with typed `get`/`set` path when bytes are compatible

Out of scope:

- Prefix iteration helpers.
- Storage trait changes.

## PR3: Raw Vec APIs (`StateVec`)

Scope:

- Add `#[cfg(feature = "native")]` raw methods on `NamespacedStateVec`:
  - `get_raw`
  - `set_raw`
  - `push_raw`
  - `pop_raw`
  - `iter_raw`
- Keep length decoded via existing `len_value` typed `u64` path.

Primary files:

- `crates/module-system/sov-modules-api/src/containers/vec.rs`

Tests:

- Add/extend vec tests for:
  - push/get/pop raw behavior
  - `iter_raw` forward/backward traversal
  - length correctness after raw operations

Out of scope:

- Storage prefix iteration changes.

## PR4: NativeStorage Kernel Prefix Iteration (NOMT)

Scope:

- Extend `NativeStorage` with `maybe_iter_kernel_values_with_prefix`.
- Implement for NOMT prover storage by plumbing `historical_state.iter_kernel_values_with_prefix`.
- Keep JMT implementation optional/`None` (same pattern as current user iterator support).

Primary files:

- `crates/module-system/sov-state/src/storage.rs`
- `crates/module-system/sov-state/src/nomt/prover_storage.rs`
- `crates/module-system/sov-state/src/prover_storage.rs`
- `crates/module-system/sov-state/src/zk_storage.rs`
- `crates/module-system/sov-state/src/nomt/zk_storage.rs`

Tests:

- NOMT native tests validating kernel-prefix iterator returns expected live pairs and ignores tombstones.

Out of scope:

- Accessory prefix iteration.
- Container-level iter helpers.

## PR5: Map Raw Iteration Helpers (User + Kernel)

Scope:

- Add `#[cfg(feature = "native")]` `iter_raw` on `NamespacedStateMap` for user/kernel namespaces.
- Iterator item should include typed key and raw bytes.
- Decode keys from `SlotKey::without_prefix()` via key codec and return structured errors on decode failures.

Primary files:

- `crates/module-system/sov-modules-api/src/containers/map.rs`

Tests:

- Map raw iter tests for user and kernel:
  - correct key decoding
  - raw value passthrough
  - decode error behavior

Out of scope:

- Accessory prefix scanning.

## PR6: Key-Driven Fallback Iteration + Script Docs

Scope:

- Add fallback helper(s) that iterate from caller-provided typed keys and return raw values:
  - `iter_raw_from_keys(...)` on map
  - optional convenience helpers for vec/value script ergonomics
- This covers accessory use cases without DB prefix index changes.
- Add migration-script-focused documentation and example(s) for offline NOMT workflows.

Primary files:

- `crates/module-system/sov-modules-api/src/containers/map.rs`
- `docs/` (new or existing scripting/migration doc location)
- Optional small example under `examples/` or utility crate if preferred

Tests:

- Accessory raw migration scenario using key-driven iteration.
- End-to-end migration test with incompatible value bytes -> transformed bytes.

Out of scope:

- Accessory DB schema/index additions.

## PR7 (Optional): Accessory Prefix Scan Support

Scope:

- Add efficient prefix iteration for accessory storage (requires new indexing/schema work in `sov-db`).
- Expose accessor-level/native-storage API for accessory prefix scans.
- Wire map `iter_raw` support for accessory namespace.

Primary files:

- `crates/full-node/sov-db/src/accessory_db.rs`
- `crates/full-node/sov-db/src/schema/...`
- `crates/module-system/sov-state/...`
- `crates/module-system/sov-modules-api/src/containers/map.rs`

Tests:

- Accessory prefix iteration correctness and historical/version semantics.
- Migration test parity with user/kernel iter behavior.

Out of scope:

- None; this is the completion phase for full parity.

## Execution Notes

1. Keep PR1-PR3 independent and mergeable first; they unlock raw migrations even without prefix scans.
2. Keep PR4 isolated to storage trait + NOMT plumbing to minimize risk from trait signature churn.
3. Start script examples/docs in PR6 only after APIs stabilize from PR1-PR5.
4. Run `cargo nextest` with package scoping per PR; use `SKIP_GUEST_BUILD=1` for local cycles.

## Acceptance Criteria

1. A script can migrate a container whose old value bytes are no longer decodable by current type/codec.
2. Keys remain strongly typed in script code.
3. Values are accessible as raw `Vec<u8>` throughout read/transform/write.
4. `StateVec` length remains decoded and stable.
5. Offline NOMT workflows are supported end-to-end with native-only APIs.
