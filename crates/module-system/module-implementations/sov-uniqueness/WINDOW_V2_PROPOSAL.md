# Window V2 Nonce: Transaction Uniqueness Proposal for Bullet

## Problem Statement

Bullet targets 5,000+ TPS globally with sub-millisecond execution (~250us per transaction in the sequencer). Market makers require hundreds of in-flight orders across multiple markets, with batch operations of up to 10 order placements/cancellations per transaction.

The current generation-based uniqueness approach (`BTreeMap<u64, HashSet<TxHash>>`) consumes **30-40% of the 250us execution budget** on Borsh deserialization and uniqueness checks alone. At 1,000 stored transaction hashes, a single serde round-trip costs **132us** — over half the total execution budget just for replay protection.

This document proposes Window V2, a fixed-size bitfield approach that reduces the full uniqueness hot path to **48ns per transaction** — a **433x improvement** over generations.

---

## Industry Landscape

| Approach | Used By | In-Flight TXs | Storage/Account | Failure Isolation |
|---|---|---|---|---|
| Sequential Nonce | Ethereum | 1 | 8 B | None — one stuck TX blocks all |
| Recent Blockhash | Solana | Unlimited | 0 (global) | Full — 60s expiry window |
| Generation Numbers | Bullet (current) | ~1,700 | Up to 75 KB | Good — per-generation buckets |
| Top-K Nonces | Hyperliquid | 100 | ~3.2 KB | Good — 2-day time window |
| **Window V2** | **Proposed** | **1,024** | **136 B (constant)** | **Good — sliding bitfield** |

### Why Ethereum Nonces Don't Work

Strict sequential ordering means a single stuck transaction blocks all subsequent ones. For a market maker quoting on 50 markets with 10 price levels each, this is a non-starter. We tried this; it was, in practice, unusable for trading clients.

### Why Generations Are Too Slow

Generations store full 32-byte transaction hashes in a `BTreeMap<u64, HashSet<TxHash>>`. This data structure:
- Requires O(G * H) deserialization on every state access (G=generations, H=hashes per generation)
- Allocates heap memory for BTreeMap nodes and HashSet buckets on every deser
- Grows to 75KB at capacity (1,700 hashes), making serde the dominant cost
- Is vulnerable to hash flooding attacks on the HashSet

### Why PR #2633 (Window V1) Isn't Enough

The original window proposal uses `(u64, Vec<u8>)` — a start offset plus a dynamic byte-vector bitfield. While conceptually sound, it has critical production issues:
- **Unbounded storage growth**: A nonce of `start + 1,000,000` allocates 125KB; `u64::MAX` causes OOM
- **Heap allocation on hot path**: `Vec::split_off` and `resize` on every mark operation
- **No upper bound on nonce jumps**: check() accepts any `nonce >= start` without limit
- **32-bit truncation**: `(nonce - start) as usize` silently truncates on wasm32/zkVM targets
- **Variable storage size**: Makes gas metering unpredictable

---

## Window V2 Design

### Data Structure

```rust
const WINDOW_BITS: usize = 1024;
const WINDOW_U64S: usize = WINDOW_BITS / 64;  // 16

struct WindowV2State {
    start: u64,               // 8 bytes — lowest tracked nonce
    bits: [u64; WINDOW_U64S], // 128 bytes — fixed bitfield
}
// Total: 136 bytes per account. Always. Regardless of usage.
```

### Core Algorithm

**check(nonce)** — O(1), zero allocation, read-only:
```
if nonce < start                    → reject "too old"
if nonce - start >= 1024            → reject "too far ahead"
if bit at position (nonce - start)  → reject "duplicate"
else                                → accept
```

**mark(nonce)** — O(1) amortized, zero allocation:
```
if nonce < start                    → reject "too old"
if nonce - start >= 1024            → reject "too far ahead"
if nonce - start >= 768             → slide window forward (O(16) shift)
set bit at position (nonce - start)
```

**shift_right(n)** — Slides the window forward by `n` bits:
- Single forward pass over 16 u64 words
- Combines word-level shift + bit-level shift in one operation
- 128 bytes of data touched — fits in 2 cache lines

### Key Design Decisions

1. **Fixed `[u64; 16]` vs `Vec<u8>`**: Eliminates all heap allocation. Borsh serialization is a flat 128-byte read/write with no length prefix processing. Deserialization is essentially `memcpy`.

2. **Reject nonces outside window**: Both `check()` and `mark()` enforce the same `[start, start + 1024)` acceptance window. An attacker cannot force window advancement by submitting a far-ahead nonce — they're rejected before any state mutation.

3. **Auto-advance at 3/4 threshold**: When a nonce lands at position >= 768, the window slides forward to center it at position 512. This maintains ~512 bits of headroom for future nonces and ~512 bits of lookback for recent ones.

4. **Range check in u64 before usize cast**: `delta_u64 >= WINDOW_BITS as u64` is checked before `delta_u64 as usize`, preventing silent truncation on 32-bit targets (wasm32, zkVM).

5. **Overflow-safe advancement**: `self.start.checked_add(advance)` prevents silent wrap-around near `u64::MAX`.

---

## Benchmark Results

**Machine**: Apple Silicon (release mode, criterion 30 samples, 2s measurement)

### Full Hot Path: Deserialize → Check → Mark → Serialize

This is **the** number that matters — it represents the per-transaction uniqueness cost in the sequencer.

| Approach | Per-TX Cost | vs Generation | % of 250us Budget |
|---|---|---|---|
| Eth Nonce | 26 ns | 800x faster | 0.01% |
| Generation | **20,807 ns** | baseline | **8.3%** |
| Window v1 | 68 ns | 306x faster | 0.03% |
| **Window v2** | **48 ns** | **433x faster** | **0.02%** |

At realistic load (500+ stored hashes), generation serde alone costs 50-132us. Window v2's entire operation is 48ns — **over 1,000x cheaper** than generation's serde-only cost.

### Batch Throughput (check + mark, 1000 operations)

| Scenario | Nonce | Generation | Window v1 | Window v2 |
|---|---|---|---|---|
| Sequential 1000 | 509 ns | 12.5 ms | 17.4 us | **1.75 us** |
| Random 700 | N/A | 6.1 ms | 8.3 us | **698 ns** |
| Sparse (stride 100) | N/A | 100 us | 3.3 us | **167 ns** |
| Cold burst 100 | 41 ns | 133 us | 1.6 us | **160 ns** |

### Per-Operation (single check + mark, pre-populated state)

| Approach | Time |
|---|---|
| Nonce | ~1 ns |
| Generation | 5,270 ns |
| Window v1 | 23 ns |
| **Window v2** | **5 ns** |

### Serialization Round-Trip (Borsh serialize + deserialize)

| TX History | Nonce | Generation | Window v1 | Window v2 |
|---|---|---|---|---|
| 100 txs | 19 ns | 7,473 ns | 54 ns | **64 ns** |
| 500 txs | 25 ns | 49,970 ns | 42 ns | **41 ns** |
| 1,000 txs | 28 ns | **131,946 ns** | 57 ns | **65 ns** |

Window v2 serde is **constant ~50-65ns** regardless of history size. Generation grows linearly — at 1,000 hashes it's **2,000x slower**.

### Check-Only (1000 validations, read path)

| Approach | Per-Check |
|---|---|
| Nonce | 0.56 ns |
| Generation | 2,428 ns |
| Window v1 | 0.50 ns |
| **Window v2** | **0.50 ns** |

### Storage Per Account (Borsh serialized bytes)

| TX Count | Nonce | Generation | Window v1 | Window v2 |
|---|---|---|---|---|
| 0 | 8 B | 4 B | 12 B | **136 B** |
| 10 | 8 B | 444 B | 14 B | **136 B** |
| 100 | 8 B | 4,404 B | 25 B | **136 B** |
| 500 | 8 B | 22,004 B | 75 B | **136 B** |
| 1,000 | 8 B | 44,004 B | 137 B | **136 B** |
| 1,700 | 8 B | **74,804 B** | 138 B | **136 B** |

Window v2 is **always 136 bytes**. At full capacity, generation is **550x larger**.

---

## Security Analysis

### Threat Model

The uniqueness mechanism is a security-critical component: any bypass enables transaction replay, which can drain accounts or double-execute trades. The sequencer processes untrusted transactions from external clients.

### Addressed Attack Vectors

| Vector | Generation | Window v1 | Window v2 |
|---|---|---|---|
| **Storage DoS** | 75KB/account | **Unbounded (OOM)** | Fixed 136B |
| **Hash flooding** | HashSet vulnerable | N/A | N/A |
| **Nonce jump → OOM** | N/A | **Critical: any nonce grows Vec** | Rejected at check() |
| **32-bit truncation** | N/A | **Bypass on wasm32** | Safe: u64 range check first |
| **check/mark mismatch** | N/A | **mark() accepts wider range** | Same policy in both |
| **start overflow** | N/A | Unchecked add | checked_add with error |
| **Replay after advance** | Possible via prune | Possible via GC | Rejected as "too old" |
| **Gas unpredictability** | Variable 4B-75KB state | Variable 12B-unlimited | Constant 136B |

### Security Properties

1. **Replay protection is permanent**: Once a nonce is used, it is either tracked in the bitfield or below `start` (rejected as "too old"). There is no path to re-accept a previously used nonce.

2. **Window advancement cannot be forced by an attacker**: Both `check()` and `mark()` reject nonces outside `[start, start + 1024)`. An attacker cannot submit a far-ahead nonce to expire other nonces' protection bits.

3. **Auto-advance preserves recent history**: When the window slides, it advances by `delta - 512`, preserving the most recent 512 nonces. A nonce at position 768 triggers an advance of only 256 positions.

4. **No heap allocation on any path**: The fixed `[u64; 16]` array eliminates allocation-based DoS entirely. No `Vec::resize`, no `split_off`, no `HashMap` bucket allocation.

5. **Constant-time operations**: Both `check()` and `mark()` are O(1) with no data-dependent branching on nonce values (beyond the range checks). The `shift_right()` operation is O(16) regardless of shift amount.

### Remaining Considerations

- **Self-griefing via window advance**: A user can advance their own window by using nonces near the top of the range, expiring their own old nonces. This is self-inflicted and each account has an independent window.
- **Counter saturation**: At u64::MAX the account is locked. At 5,000 TPS this takes ~117 million years. `checked_add` prevents silent overflow.
- **1,024 in-flight limit**: An account can track at most 1,024 nonces simultaneously. This is ample for market-making. If more are needed, Bullet's signer delegation feature allows splitting across multiple signers.

---

## Production Readiness Assessment

### Ready

- Core algorithm is correct (22 unit tests, including security-specific tests)
- All critical security issues from initial review have been fixed and verified
- Performance exceeds requirements by orders of magnitude
- Zero heap allocation on hot path
- Constant, predictable storage and gas costs
- Borsh serialization is trivial (flat 136-byte read/write)

### Required Before Production Deployment

1. **Extract to `src/window_v2.rs`**: The implementation currently lives in benchmark/test files with duplication. Promote to a proper source module with `pub(crate)` visibility, imported by both bench and test.

2. **Integrate into `Uniqueness` module**: Add `StateMap<CredentialId, WindowV2State>` to the `Uniqueness` struct. Wire into `capabilities.rs` dispatch. Add `UniquenessData::WindowV2(u64)` variant or replace the existing `Window` variant.

3. **Add `next_window()` API**: Expose a public method for clients to query the current window state (start + next available nonce), analogous to `next_nonce()` and `next_generation()`.

4. **Client SDK integration**: Update the Bullet SDK to support window v2 nonce selection. Recommended client strategy: maintain an atomic counter starting from the last known `start` value, incrementing per transaction.

5. **Migration path**: If existing accounts use generations, define a migration strategy (e.g., new accounts default to window v2, existing accounts can opt-in via a transaction).

6. **Consider const generic window size**: `WindowV2State<const N: usize>` would allow different deployments to tune the window size without code changes. 1024 is the recommended default.

### Not Required (Non-Issues)

- **`#[inline]` hints**: Already added to `check()`, `mark()`, and `shift_right()`.
- **Concurrent access**: The blockchain execution model is sequential per account. No locking needed.
- **Backwards compatibility**: This is a new variant alongside existing nonce/generation modes, not a replacement.

---

## Recommendation

**Ship Window V2 as the default uniqueness mechanism for Bullet.**

The performance improvement is transformative: from 8.3% of the execution budget (generation) to 0.02% (window v2). For a system targeting sub-millisecond execution at 5,000 TPS, eliminating a 20us-per-transaction bottleneck is the difference between hitting latency targets and not.

The security properties are strictly stronger than both generations (no hash flooding, no variable storage DoS) and window v1 (no OOM, no 32-bit truncation, no check/mark mismatch). The fixed 136-byte storage makes gas metering fully predictable.

The 1,024 nonce window is sufficient for the target workload (hundreds of in-flight orders per market maker), and Bullet's signer delegation provides a natural scaling path if more parallelism is needed.
