# sov-celestia-adapter Production Readiness Audit Plan

## Audit Scope

**Goal**: Catalog all issues preventing production readiness with priorities for remediation roadmap.

**Target versions**:
- celestia-node `v0.28.2`
- celestia-app `v6.2.2`

**Trust model**:
- Native service may trust its RPC for availability, but the **verifier must treat all inputs as untrusted**, including any data assembled by the native service.

**Coverage targets**:
- 100% line + branch coverage (cargo-llvm-cov)
- Property-based testing (proptests) for all input paths and invariants
- Fuzz testing for parsing and verification logic (panic-free)
- Prioritize proof correctness, completeness, and DoS-resilience

---

## 1. Panic Audit - Issue Catalog

### 1.1 Critical: Verifier Panics (ZK + Native)

| # | File | Line | Code | Issue | Priority |
|---|------|------|------|-------|----------|
| P1 | verifier/mod.rs | 185 | `expect("Bug: caller didn't set last_validated_share_idx")` | Logic invariant can fail on malformed input | CRITICAL |
| P2 | verifier/mod.rs | 234-235 | `expect("At least 1 share should be proven by this point")` | Empty/invalid proofs still panic | CRITICAL |
| P3 | verifier/mod.rs | 359 | `namespace_row_roots[row_number]` | Unchecked index from untrusted proof indices | CRITICAL |
| P4 | verifier/mod.rs | 417-419 | `assert_eq!(shares_proven, num_shares_to_prove)` | Assertion on untrusted proof data | HIGH |
| P5 | verifier/mod.rs | 425 | `expect("sequence length should be set by this point")` | Missing sequence length on v1 share | CRITICAL |
| P6 | verifier/mod.rs | 449-451 | `assert!(payload_all_zeros.is_some())` | Padding share validation panics | HIGH |
| P7 | verifier/mod.rs | 454-456 | `assert_ne!(info_byte.version(), SUPPORTED_SHARE_VERSION)` | Version mismatch assertion | HIGH |
| P8 | verifier/mod.rs | 468 | `namespace_row_roots[row_number]` | Unchecked index in skipped-blob path | CRITICAL |
| P9 | verifier/mod.rs | 500 | `assert!(last_proven_share_idx <= last_share_last_row_idx)` | Arithmetic invariant on untrusted inputs | CRITICAL |
| P10 | verifier/mod.rs | 514-518 | `expect("Square overflow")` | u64 overflow on large squares | CRITICAL |
| P11 | verifier/mod.rs | 534-536 | `expect("Empty namespace row roots have been checked before")` | Pre-condition invariant | HIGH |
| P12 | verifier/proofs.rs | 127-129 | `expect("Share index overflow")` | Untrusted proof index overflow | HIGH |

### 1.2 High: Native Proof/Extraction Panics (RPC-derived data)

| # | File | Line | Code | Issue | Priority |
|---|------|------|------|-------|----------|
| P13 | verifier/proofs.rs | 180-185 | `assert!(relevant_end <= range.end)` | Range validation assertion | HIGH |
| P14 | verifier/proofs.rs | 236-239 | `expect("Bug. Missing info byte/sequence length")` | Malformed share from RPC | HIGH |
| P15 | verifier/proofs.rs | 242-247 | `assert!(payload_all_zeros.is_some())` | Padding validation panics | HIGH |
| P16 | verifier/proofs.rs | 249-253 | `assert_ne!(info_byte.version(), SUPPORTED_SHARE_VERSION)` | Version assertion | HIGH |
| P17 | verifier/proofs.rs | 314-315 | `expect("Row cannot be larger that square size")` | Size validation panic | MEDIUM |
| P18 | verifier/proofs.rs | 330-353 | `unwrap()` / `checked_*` | Row-range math can panic on malformed inputs | HIGH |
| P19 | verifier/proofs.rs | 363 | `row_roots[row_num]` | Unchecked index (row roots mismatch) | HIGH |
| P20 | verifier/proofs.rs | 372 | `expect("invalid proof self-check")` | Proof self-check panic | HIGH |
| P21 | verifier/proofs.rs | 383 | `panic!("Empty proof for blob range")` | Empty proof case | HIGH |
| P22 | verifier/proofs.rs | 428-432 | `expect("Index overflow")` | Overflow in range splitting | HIGH |
| P23 | verifier/proofs.rs | 440-441 | `expect("square_size cannot be 0")` | Division by zero | HIGH |
| P24 | verifier/proofs.rs | 460-463 | `expect("Row index overflow")` | Multiplication overflow | HIGH |
| P25 | types/mod.rs | 269-286 | `panic!(..)` / `expect(..)` | NamespaceBoundaryProof construction panics | HIGH |

### 1.3 High: Share/Blob Parsing Panics

| # | File | Line | Code | Issue | Priority |
|---|------|------|------|-------|----------|
| P26 | shares.rs | 109-116 | `self.0[0]` / `expect(..)` | Missing start share or parity payload | HIGH |
| P27 | shares.rs | 142-147 | `self.blob.0[self.current_idx]` | OOB when sequence_len > share count | HIGH |
| P28 | shares.rs | 168-173 | `expect("BlobIterator has consumed more than available bytes")` | Underflow in remaining() | HIGH |
| P29 | shares.rs | 209-215 | `expect(PARITY_SHARE_PANIC)` | Parity share payload access | HIGH |
| P30 | shares.rs | 219-222 | `assert_eq!(remaining_in_current_share, cnt)` | Buf advance invariant | MEDIUM |
| P31 | types/mod.rs | 180-186 | `expect("blob must be valid")` | Commitment creation panic | HIGH |

### 1.4 Medium: Header/Service Panics

| # | File | Line | Code | Issue | Priority |
|---|------|------|------|-------|----------|
| P32 | celestia.rs | 84-95 | `unwrap()` on header optionals | Malformed header fields | MEDIUM |
| P33 | celestia.rs | 183 | `expect("square_width must be divisible by 2")` | Invalid DAH | MEDIUM |
| P34 | celestia.rs | 191 | `expect("the square size is invalid")` | Division by zero | MEDIUM |
| P35 | celestia.rs | 234 | `lock().unwrap()` | Poisoned mutex | LOW |
| P36 | celestia.rs | 242-243 | `expect("header must be validly encoded")` | Protobuf decode failure | MEDIUM |
| P37 | celestia.rs | 254-257 | `expect("must not call prev_hash...")` | Genesis/invalid last_block_id | LOW |
| P38 | celestia.rs | 268-275 | `expect("Height/Timestamp must be valid")` | Header field parsing | MEDIUM |
| P39 | da_service/mod.rs | 88 | `panic!("Passed unknown namespace")` | Config error | MEDIUM |
| P40 | da_service/mod.rs | 115-116 | `expect("Bug in CelestiaAdapter")` | Blob creation | MEDIUM |
| P41 | da_service/mod.rs | 196-199 | `expect("Failed to build celestia-client")` | Startup failure | MEDIUM |

**Total: 41 panic points identified (update as audit progresses)**

---

## 2. Security Review - Attack Vectors

### 2.1 README Spec Requirements (Must verify each)

| # | Requirement | Implementation | Test Status | Priority |
|---|-------------|----------------|-------------|----------|
| S1 | Reject blob order modifications | `verify_continuity()` in `verifier/proofs.rs` | **MISSING TEST** | CRITICAL |
| S2 | Reject sender tampering | `authenticate_blob_data()` in `verifier/mod.rs` | Has test | VERIFY |
| S3 | Reject blob omission | `check_namespace_end_boundary()` in `verifier/mod.rs` | Has test | VERIFY |
| S4 | Reject blob duplication | Range checks in `verify_continuity()` | **MISSING TEST** | CRITICAL |
| S5 | Reject extra blobs | `prevalidate_blobs()` + proofs | **PARTIAL** (no explicit test) | HIGH |

### 2.2 Additional Attack Vectors to Test

| # | Attack Vector | Location | Test Status | Priority |
|---|--------------|----------|-------------|----------|
| S6 | Blob insertion (fake blob in proof) | Row proof verification | **MISSING** | CRITICAL |
| S7 | Index manipulation (wrong start_share_idx) | `verify_left_boundary()` / `verify_continuity()` | **MISSING** | CRITICAL |
| S8 | Row root index out of bounds | `namespace_row_roots[row_number]` | **MISSING** | CRITICAL |
| S9 | Namespace confusion (wrong namespace in proof) | NMT verification | **MISSING** | HIGH |
| S10 | Proof reordering (swap proof order) | `verify_continuity()` | **MISSING** | HIGH |
| S11 | Left boundary skip (not at namespace start) | `verify_left_boundary()` | **MISSING** | CRITICAL |
| S12 | Right boundary skip (missing end proof) | `check_namespace_end_boundary()` | **MISSING** | CRITICAL |
| S13 | Gap between blobs (non-contiguous shares) | `verify_continuity()` | **MISSING** | CRITICAL |
| S14 | Row root manipulation | Row proof verification | **MISSING** | HIGH |
| S15 | Share version spoofing | `is_supported_blob()` | **MISSING** | MEDIUM |
| S16 | Tail padding contains non-zero bytes | `verify_skipped_blob()` | **MISSING** | HIGH |
| S17 | Sequence length overflow / truncation | `shares_needed_for_bytes(sequence_length as usize)` | **MISSING** | HIGH |
| S18 | BlobIterator OOB via oversized sequence length | `BlobIterator::next()` | **MISSING** | HIGH |

### 2.3 Unsafe Code

| # | File | Line | Code | Issue | Priority |
|---|------|------|------|-------|----------|
| U1 | verifier/address.rs | 125 | `unsafe { std::str::from_utf8_unchecked(value) }` | Guarded by is_ascii() but unsafe unnecessary | LOW |

---

## 3. Test Coverage Expansion

### 3.1 Missing Adversarial Tests (CRITICAL)

Create in `da_service/tests.rs`:

```rust
// S1: Reject blob order modifications
#[test]
fn verification_fails_if_blob_order_swapped() {}

// S4: Reject blob duplication
#[test]
fn verification_fails_if_blob_duplicated() {}

// S6: Reject fake blob insertion
#[test]
fn verification_fails_if_fake_blob_inserted() {}

// S10: Left boundary attack
#[test]
fn verification_fails_if_left_boundary_missing() {}

// S11: Right boundary attack
#[test]
fn verification_fails_if_right_boundary_missing() {}

// S12: Gap attack
#[test]
fn verification_fails_if_gap_between_blobs() {}

// S7: Index manipulation
#[test]
fn verification_fails_if_start_index_manipulated() {}

// S8: Namespace confusion
#[test]
fn verification_fails_for_wrong_namespace_proof() {}

// S9: Proof reordering
#[test]
fn verification_fails_if_proofs_reordered() {}
```

### 3.2 Edge Case Tests (HIGH)

```rust
#[test]
fn test_empty_namespace_with_presence_in_row_root() {}

#[test]
fn test_blob_exactly_fills_row() {}

#[test]
fn test_blob_spans_multiple_rows() {} // 3+ rows

#[test]
fn test_max_square_size_blob() {}

#[test]
fn test_single_byte_blob() {}

#[test]
fn test_blob_with_all_zeros() {}

#[test]
fn test_v0_blob_correctly_skipped() {}

#[test]
fn test_mixed_v0_v1_blobs_ordering() {}

#[test]
fn test_unsupported_version_handling() {} // V2+ future-proofing

#[test]
fn test_row_root_index_out_of_bounds_is_error() {}

#[test]
fn test_proof_start_index_out_of_bounds_is_error() {}

#[test]
fn test_tail_padding_non_zero_rejected() {}

#[test]
fn test_missing_sequence_length_in_v1_share() {}

#[test]
fn test_sequence_length_overflow_rejected() {}

#[test]
fn test_blob_iterator_oob_rejected() {}
```

### 3.3 Property-Based Tests (Proptests) - Priority

Add to `crates/adapters/celestia/src/` test modules (new `proptests.rs` or alongside existing tests):
* Reuse existing proptest strategies in `shares.rs` (`share_bytes_strategy`, `blob_strategy`) and wire them into actual tests.

```rust
use proptest::prelude::*;

// Verifier should never panic on structured but adversarial inputs
proptest! {
    #[test]
    fn proptest_verify_never_panics(
        // Generate small structured inputs and mutate them.
        // Use existing test_data JSON as a corpus and apply random edits.
        // Expect Result::Err, never panic.
    ) {
    }
}

// Share parsing robustness (arbitrary bytes)
proptest! {
    #[test]
    fn proptest_share_parsing_never_panics(
        share_bytes in prop::collection::vec(any::<u8>(), 0..2048),
    ) {
        // Try celestia_types::Share::from_raw and BlobIterator creation
        // Should return Err or empty, never panic
    }
}

// Proof range splitting invariants (no overlap, full coverage)
proptest! {
    #[test]
    fn proptest_split_blob_range_coverage(
        start in 0usize..10_000,
        len in 1usize..10_000,
        offset in 0usize..256,
        square_size in 1usize..512,
    ) {
        // Verify split_blob_range_by_rows handles all inputs without panic
    }
}

// Namespace boundary verification should never panic
proptest! {
    #[test]
    fn proptest_namespace_boundary_any_index(
        last_proven_idx in 0usize..10_000,
        row_roots_count in 1usize..512,
        row_length in 1usize..1024,
    ) {
        // Expect Result::Err, never panic
    }
}

// BlobIterator + CountedBufReader should not panic on partial reads
proptest! {
    #[test]
    fn proptest_blob_iterator_partial_reads(
        payload in prop::collection::vec(any::<u8>(), 0..4096),
        read_len in 0usize..8192,
    ) {
        // Build a v1 blob and read read_len bytes through CountedBufReader
    }
}
```

### 3.4 Fuzz Testing Targets (crates/fuzz) - Priority

Add new targets under `crates/fuzz/fuzz_targets/` and register them in `crates/fuzz/Cargo.toml`:
* Add `sov-celestia-adapter` as a fuzz dependency with `features = ["native", "arbitrary"]` to reuse types and helpers.

```rust
// crates/fuzz/fuzz_targets/celestia_verify_blobs.rs
fuzz_target!(|data: &[u8]| {
    // Use corpus-based inputs from crates/adapters/celestia/test_data/*
    // Apply random mutations to headers/shares/proofs
    // Call CelestiaVerifier::verify_relevant_tx_list; must not panic
});

// crates/fuzz/fuzz_targets/celestia_parse_share.rs
fuzz_target!(|data: &[u8]| {
    // Share::from_raw + BlobIterator + CountedBufReader
    // Must not panic
});

// crates/fuzz/fuzz_targets/celestia_blob_iterator.rs
fuzz_target!(|data: &[u8]| {
    // Construct malformed share sequences to stress BlobIterator::next/advance/remaining
});

// crates/fuzz/fuzz_targets/celestia_header_parsing.rs
fuzz_target!(|data: &[u8]| {
    // Parse as CompactHeader/CelestiaHeader (postcard/bincode)
    // validate_dah and BlockHeaderTrait methods should not panic
});
```

### 3.5 Network Error Tests (Currently Ignored)

Enable with mock server or toxiproxy:

- `test_submit_blob_application_level_error` (line 127)
- `test_submit_blob_internal_server_error` (line 148)
- `test_submit_blob_response_timeout` (line 173)

---

## 4. Error Type Expansion

### 4.1 Missing Error Variants

Add to `types/error.rs`:

```rust
pub enum NamespaceValidationError {
    // Existing...

    // NEW: For panic replacement
    #[error("Share index overflow")]
    IndexOverflow,

    #[error("Row index out of bounds")]
    RowIndexOutOfBounds,

    #[error("Square calculation overflow")]
    SquareOverflow,

    #[error("Invalid share structure: {0}")]
    InvalidShareStructure(String),

    #[error("Sequence length overflow or truncation")]
    SequenceLengthOverflow,

    #[error("Tail padding contains non-zero bytes")]
    TailPaddingNonZero,

    #[error("Unsupported share version: {0}")]
    UnsupportedShareVersion(u8),
}

pub enum ValidationError {
    // Existing...

    // NEW: For panic replacement
    #[error("Invalid square size in DAH")]
    InvalidSquareSize,

    #[error("Malformed block header: {0}")]
    MalformedHeader(String),
}
```

---

## 5. Input Validation Gaps

| # | Input | Location | Current | Needed | Priority |
|---|-------|----------|---------|--------|----------|
| V1 | rpc_url | config.rs:12-13 | None | URL format validation | LOW |
| V2 | grpc_url | config.rs:22-24 | None | URL format validation | LOW |
| V3 | namespace params | verifier/mod.rs:73-78 | None | Reserved namespace check | MEDIUM |
| V4 | Share version | types/mod.rs:22 | Only v1 | Future version handling | HIGH |
| V5 | Parity namespace | mod.rs:329-332 | Checked | Also check PADDING, TAIL_PADDING, PAY_FOR_BLOB | MEDIUM |
| V6 | sequence_length -> usize | verifier/mod.rs:425, 475 | Cast without bounds | Overflow/truncation check | HIGH |
| V7 | row_roots index | verifier/mod.rs:359, 468 | Unchecked index | Bound check + error | HIGH |
| V8 | tail padding bytes | verifier/mod.rs:447-452 | Only `is_some()` | Enforce all-zero payload | HIGH |
| V9 | share_count vs sequence_length | shares.rs:142-147 | Assumes consistent | Validate before BlobIterator | HIGH |

---

## 6. Critical Files Summary

| File | Issues | Tests Needed |
|------|--------|--------------|
| `verifier/mod.rs` | P1-P11 (11 panics) | All adversarial tests + proptests |
| `verifier/proofs.rs` | P12-P24 (13 panics) | Proptests + fuzzing (range splitting, index bounds) |
| `shares.rs` | P26-P30 (5 panics) | Fuzz targets for parsing + BlobIterator |
| `types/mod.rs` | P25, P31 | Fuzz for boundary proof + blob commitment |
| `celestia.rs` | P32-P38 (7 panics) | Header parsing fuzz |
| `da_service/mod.rs` | P39-P41 (3 panics) | Network error tests |

---

## 7. Verification Plan

After remediation, verify with:

1. **Run existing tests**: `SKIP_GUEST_BUILD=1 cargo test -p sov-celestia-adapter --all-features`
2. **Check coverage (line + branch)**: `SKIP_GUEST_BUILD=1 cargo llvm-cov nextest -p sov-celestia-adapter --features native --html`
3. **Run proptests**: `PROPTEST_CASES=50 SKIP_GUEST_BUILD=1 cargo test -p sov-celestia-adapter --features native -- --ignored proptest`
4. **Run fuzz tests** (from `crates/fuzz`): `cargo +nightly fuzz run celestia_verify_blobs -- -runs=100000`
5. **Integration test**: Start devnet docker, run `test_service_starts` or existing submit/receive tests

---

## 8. Priority Matrix

### Phase 1: Critical Security (Immediate)
- [ ] P1-P12 - Verifier panics on untrusted inputs
- [ ] S1, S4, S6-S13 - Missing adversarial tests for ordering/completeness/indexing
- [ ] Add error variants for panic replacement
- [ ] Proptest/fuzz harnesses that assert "never panic" in verifier

### Phase 2: High Priority
- [ ] P13-P31 - Native proof/extraction + share parsing panics
- [ ] S14-S18 - Additional attack vector tests (padding/version/overflow)
- [ ] Proptest coverage for all input paths + structured generators

### Phase 3: Medium Priority
- [ ] P32-P41 - Header/service panics
- [ ] Fuzz infrastructure hardening (corpus, minimize, CI smoke run)
- [ ] Input validation improvements
- [ ] Network error test enablement

### Phase 4: Polish
- [ ] U1 - Remove unnecessary unsafe
- [ ] V1, V2 - URL validation
- [ ] Documentation updates (Celestia version v0.28.2/v6.2.2)
- [ ] Remove "research-only" warning from README

---

## Appendix: Test Data Requirements

For adversarial tests, need to generate:
1. Blocks with swapped blob order
2. Blocks with duplicated blobs
3. Blocks with fake blob insertion
4. Blocks with missing left boundary proof
5. Blocks with missing right boundary proof
6. Blocks with gaps between blobs
7. Blocks with manipulated start indices
8. Blocks with wrong namespace proofs
9. Blocks with out-of-bounds row/root indices
10. Blocks with non-zero tail padding
11. Blocks with oversized sequence_length (overflow/truncation)

Can generate using existing test helper infrastructure in `test_helper/files.rs` by modifying serialized test data.
