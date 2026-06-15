# Celestia Blob Compression — Actual Plan & Progress (v4)

Status: ACTIVE — reworked from v3 after a security + effort review (2026-06-15). Two
changes from v3: (1) the envelope is **chunked from the start** (not a single LZ4 frame),
to preserve the SDK's partial-read capability; (2) the `decode` contract is **corrected**
— it must never emit logical bytes for un-authenticated input (the v3 "zero-fill to
`logical_len` on short/error" rule was unsafe under an adversarial prover; see Fact C).
Scope: `crates/adapters/celestia` (self-contained; no public API changes; verifier
unchanged).
Supersedes: `celestia-compression-{claude,claude-2,codex,codex-2}.md` (kept for history).

## Decisions that frame this design (2026-06-15)

- **Chunked envelope from the start.** Independently-decodable fixed-size logical chunks,
  not a single frame. Rationale: preserve the partial-read capability `BlobReaderTrait`/
  `CountedBufReader` were built around (see "Design intent"), and bound decode work
  per-chunk. **Honest caveat:** Fact A says no *current* STF caller does partial reads, so
  chunking buys no benefit *today*; it is forward-looking, and it enlarges the
  consensus-critical/audit surface (disadvantage #1). Single-frame + the Fact-C decode fix
  would be ~5–7 days cheaper and a smaller audit, at the cost of the partial-read property.
- **Prover is adversarial toward sequencers.** A malicious prover must not be able to
  slash an honest sequencer, nor attest an invalid transition as valid. This is what makes
  Fact C a must-fix rather than cosmetic.

## Progress

| PR | Content | Status |
|----|---------|--------|
| PR1 | Accessor split (`compressed_*`/`logical_*`) + `TotalLenMismatch` security fix | ✅ **Committed** (`43fb15f92`) |
| PR2 | Chunked envelope read path: `envelope.rs` + chunk-aware decode-on-read on `BlobWithSender` (dark; nothing emits envelopes) | ⏳ Not started |
| PR3 | Submit path (chunked encode) + `compress_on_submit` config + metrics + e2e | ⏳ Not started |

**Dropped:** the earlier "witness-pruning via a `Native`/`Verified` enum + custom serde" PR. It was over-engineered, and the `Verified` label implied trust that does not exist — witness contents are attacker-controlled until authenticated. Witness-pruning is now **out of scope** (the pre-existing unread-share bloat is unrelated to compression and not worsened by it).

Process: each PR is implemented + verified in the working tree; **Nikolai reviews and commits himself**. The next PR starts only after the previous one is committed.

## Three facts that shape the design

### Fact A — reads are full-or-zero, never a partial prefix (verified in code)

A DA blob is read by the STF in exactly one of two ways:
- **Full read** — `data_for_deserialization(blob)` returns `full_data()` (native) / `verified_data()` (guest), then borsh `try_from_slice` deserializes the whole slice in one call (`sov-blob-storage/src/capabilities.rs:1059`).
- **Zero data bytes read** — the blob is skipped by a `total_len()`-only gate *before* any payload byte is read: capacity limiter (`capabilities.rs:150`), emergency-registration size gate (`:230`, `MAX_EMERGENCY_REGISTRATION_BLOB_SIZE = 1000`), gas pre-charge (`:1046`), `batch_selector`.

There is **no path that reads a non-trivial prefix and abandons the rest.** The completeness assert (`capabilities.rs:1063`, `verified_data().len() == total_len()`) fires only after a *failed* deserialization — i.e. after a full read.

**Implication for chunking:** chunking adds *no value to the current STF* (it never partial-reads). We adopt it anyway as a forward-looking capability (Design intent) and to bound per-chunk decode cost. This is a deliberate, paid-for choice, not a Fact-A requirement.

### Fact B — the verifier trusts nothing from the witness

Trust comes only from the block header and the Celestia protocol, never from prover-serialized witness data:
1. `block_header` is the trusted public input. `validate_dah()` checks `dah.hash() == header.data_hash` → binds row roots to the header.
2. `proof.verify_range(row_root, shares.map(s.data()), namespace)` (nmt-rs) authenticates **share bytes**.
3. `sequence_length` / `signer` / `payload()` from the proven first share are trusted.
4. Witness fields are authenticated **against** those shares: the consumed-bytes accumulator is **byte-compared vs `share.payload()`** (`verifier/mod.rs:385-420`); `compressed_total_len() == sequence_length` (PR1, `:451-456`); `sender == recovered signer` (`:391-396`). Shares proven `= shares_needed_for_bytes_with_signer(accumulator.len()).max(1)` (`:355-369`), matched exactly → work scales with bytes consumed.

Because the accumulator is byte-compared against `share.payload()`, and shares hold the **compressed/on-DA** bytes, **the accumulator is compressed bytes**, authenticated as-is. Logical bytes cannot be tied to shares — they are **decompressed** from the authenticated accumulator. The witness never carries logical bytes and the verifier never "compares decoded vs claimed." Proof generation uses `compressed_*` exclusively (`proofs.rs:175,188`).

### Fact C — the verifier+STF interlock that stops a truncating prover, and how compression must preserve it

Today, a malicious prover cannot slash an honest sequencer: bytes are authenticated (it cannot corrupt them), and *under-reading* (truncating) is caught by an interlock:
- the verifier pins `compressed_total_len() == sequence_length` (`verifier/mod.rs:451`) — note `total_len() = inner.remaining() + accumulator.len()`, so the prover cannot lie about the blob's full length; and
- the STF asserts `verified_data().len() == total_len()` (`capabilities.rs:1063`, *"…some data was not provided. The prover might be malicious"*).

A prover who authenticates only a 500-of-1000-byte prefix (legal — the verifier proves only `shares_needed(500)` shares against the real prefix) trips the assert (`500 != 1000`) → guest panic → no valid proof.

**Compression must not break this interlock.** The v3 `decode` rule ("force output to exactly `logical_len`, zero-fill on any short/error") *does* break it: logical `total_len()` becomes a header value decoupled from the authenticated compressed length, and `verified_data()` becomes `decode(prefix)` zero-filled back to `logical_len`, so the assert degenerates to `logical_len == logical_len` (always true). A malicious prover could then authenticate a prefix → zero-fill → borsh fails on garbage → **honest sequencer slashed with a valid proof.** The corrected decode contract below restores the interlock entirely in the read path (verifier still unchanged).

## Design

### Envelope format v1 (consensus-critical adapter constants, NOT config)

```
magic        [16] b"SOV_CELESTIA_CMP"   (ASCII, no 0x00 — share zero-padding can't fake it)
version      [1]  = 1
codec        [1]  0 = raw, 1 = LZ4 block   (2 reserved for zstd)
flags        [2]  must be 0
logical_len  [4]  u32 LE, <= MAX_LOGICAL_BLOB_LEN
-- 24-byte fixed header. num_chunks = ceil(logical_len / CHUNK_LOGICAL_LEN) is derived
-- from logical_len, so the chunk table size is known after the header.
chunk_table  [4 * num_chunks]  u32 LE compressed length of each chunk
payload      [..]  num_chunks back-to-back chunks; chunk i is `chunk_table[i]` bytes,
                   codec=1: an LZ4 block decoding to CHUNK_LOGICAL_LEN (the final chunk
                   decodes to logical_len - CHUNK_LOGICAL_LEN*(num_chunks-1)); codec=0: raw.
```
Constants: `MAX_LOGICAL_BLOB_LEN = 16 MiB` (decompression-bomb cap); `CHUNK_LOGICAL_LEN =
64 KiB` (fixed logical chunk size). Fixed logical chunk size keeps the logical→chunk map
arithmetic (`chunk = logical_offset / CHUNK_LOGICAL_LEN`) — only *compressed* chunk lengths
need storing, and the table for a 16 MiB blob is ≤256×4 B ≈ 1 KiB. The whole header +
chunk table is authenticated for free (it lies in the accumulator, which the verifier
byte-compares against the first shares).

### Mode detection — derived from the (authenticated) accumulator, never claimed

`mode = f(accumulator_prefix, sequence_length)`, one shared pure function:
- no 16-byte magic prefix (incl. prefix `< 16`) → **Legacy** (today's behavior, byte-for-byte).
- magic + valid header (version 1, known codec, flags 0, `logical_len ≤ MAX`) + a chunk table consistent with `logical_len` → **Envelope**.
- magic + invalid/oversized header or inconsistent table → **InvalidEnvelope** → behaves as an empty blob (`total_len()=0`, no data).

### Deterministic, total, chunk-aware decode (the corrected logic — the only new logic)

`decode(accumulator, compressed_total_len) -> Vec<u8>`, identical native and guest, never
fails (never errors the verifier — rejecting on-DA bytes would halt the chain). Two clauses,
and the split between them is what restores Fact C's interlock:

- **Full frame authenticated** (`accumulator.len() == compressed_total_len`, i.e. every
  chunk's compressed bytes are present): decode is **total** and returns **exactly
  `logical_len` bytes**. Each chunk is LZ4-decoded to its declared logical span; any chunk
  whose block is malformed / short / over-long, or any table inconsistency, is
  **zero-filled within its span**. (Garbage *content* on DA ⇒ deterministic logical bytes
  ⇒ borsh fails ⇒ existing slash path. Safe because the bytes are authenticated real on-DA
  bytes — a prover cannot manufacture this for an honest sequencer.)
- **Only a prefix authenticated** (`accumulator.len() < compressed_total_len`): return the
  logical bytes of the **maximal run of chunks fully contained in the accumulator, and no
  more** — a partial/truncated trailing chunk contributes **zero** logical bytes, never
  zero-fill. For a real truncation this is `< logical_len` ⇒ the STF completeness assert
  (`capabilities.rs:1063`) fires ⇒ the truncating prover is rejected (Fact C restored). For
  a *legitimate* future partial read it is exactly the requested logical prefix, nothing
  fabricated.

- Legacy → the accumulator unchanged (returned by reference; no copy).
- InvalidEnvelope, or header-only accumulator (zero-read) → empty.

The prover cannot cheat: the guest decodes the same authenticated accumulator the prover's
native execution consumed, so any divergence yields a different state root than committed
(invalid proof). The `MAX_LOGICAL_BLOB_LEN` and per-chunk caps live in this shared `decode`,
applied identically native and guest.

### `BlobWithSender` — one type, minimal change

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]   // derived serde; UNCHANGED wire shape
pub struct BlobWithSender {
    blob: CountedBufReader<BlobIterator>,   // the COMPRESSED stream (accumulator = consumed on-DA bytes)
    range_in_namespace: Range<usize>,
    sender: CelestiaAddress,
    hash: HexHash,
    #[serde(skip)]
    decoded: std::sync::OnceLock<Vec<u8>>,  // lazy logical-decode cache; never serialized
}
```
- **Wire/witness shape unchanged** — the only new field is `#[serde(skip)]`. PR2 is therefore **not** a breaking witness change.
- `compressed_verified_data()` = `blob.accumulator()`, `compressed_total_len()` = `blob.total_len()` — **unchanged**; verifier and proof-gen keep using these.
- `verified_data()` (logical): Legacy → `blob.accumulator()` (zero-overhead). Envelope/Invalid → `decoded.get_or_init(|| decode(blob.accumulator(), compressed_total_len()))`.
- `total_len()` (logical): Legacy → `compressed_total_len()`. Envelope → `logical_len` parsed from the authenticated header. Invalid → 0.
- `advance(n)` (native): Legacy → `blob.advance(n)`. Envelope → advance the *compressed* accumulator to cover the chunks spanning the next `n` logical bytes (`chunk = logical_offset / CHUNK_LOGICAL_LEN`; consume those chunks' compressed bytes per the table). The only caller today is `full_data()` (n = the rest), which consumes the whole frame; the chunk machinery exists so a future partial reader can stop after a malformed prefix.
- **Construction** (`get_blobs_with_sender`, native, `types/mod.rs:200-234`): peek the first share for the magic; if Envelope, eagerly `advance` the **header + chunk table** so logical `total_len()` and the chunk map are available before any read (the size gates call `total_len()` first) **and authenticated for free** — those bytes are in the accumulator, which the verifier already byte-compares against the first shares. No new field, no new verifier check, 0 extra shares for a small header+table (⊂ first shares; `.max(1)` already proves them).
- Manual `PartialEq` over `{blob, range, sender, hash}` (ignore the `decoded` cache).

### Why the verifier is unchanged

It already authenticates the compressed accumulator against shares and checks
`compressed_total_len == sequence_length` (PR1, `verifier/mod.rs:451`). Mode, `logical_len`,
the chunk map, and logical bytes are **deterministic functions of the authenticated
accumulator**, computed in the read path after verification. Crucially, Fact C's interlock
is preserved *in the read path*: `decode` emits `< logical_len` bytes whenever the frame is
not fully authenticated, so the STF completeness assert still catches a truncating prover —
no verifier change needed.

### Submit path & config (PR3)

- `CelestiaConfig.compress_on_submit: CompressOnSubmit` — `#[serde(rename_all="lowercase")] enum { #[default] Off, Lz4, Zstd }` (+ JsonSchema, following the `TxPriority` pattern in `config.rs`). Construction rejects `Zstd` ("reserved"). If activation gating is adopted (disadvantage #4), this pairs with a protocol activation height rather than standing alone.
- `send_transaction` (batch namespace): `Lz4` → `encode` (split logical into `CHUNK_LOGICAL_LEN` chunks, compress each, build header + table + chunks; fall back to passthrough if the envelope is not strictly smaller). **Always** wrap magic-prefixed payloads in a codec=0 envelope even when `Off` (keeps legacy detection unambiguous). `send_proof` → magic-escape only. `get_proofs_at_inner` → `decode` for transparency. Hash/commitment math untouched (`da_service/mod.rs:116`, commitment is over posted bytes).
- Metrics (logical vs posted bytes, saved basis points, outcome).

## PR breakdown

### PR1 — Accessor split + `total_len` auth ✅ committed (`43fb15f92`)

### PR2 — Chunked envelope read path (dark)
- New `crates/adapters/celestia/src/envelope.rs` (not cfg-gated; guest needs `decode`): constants (`MAGIC`, `MAX_LOGICAL_BLOB_LEN`, `CHUNK_LOGICAL_LEN`), `Codec`, header + chunk-table parse, `classify`, the two-clause `decode` above, `#[cfg(feature="native")] encode`, `starts_with_magic`. The chunk map + the full-frame-vs-prefix split is the bulk and the riskiest code — audit target #1.
- Dep: workspace `lz4_flex = { version = "0.11", default-features = false, features = ["std","safe-decode","safe-encode"] }`, **non-optional** (guest decode needs it; pure Rust; same version native+guest ⇒ decode determinism). Verify it compiles for both risc0 and sp1 guest targets (guest has `std`).
- `types/mod.rs`: add `#[serde(skip)] decoded` cache + manual `PartialEq`; make `verified_data()`/`total_len()`/`advance()` chunk-aware; `get_blobs_with_sender` peeks the magic and eager-advances the header + chunk table for envelopes.
- `verifier/mod.rs`, `verifier/proofs.rs`: **unchanged** (already use `compressed_*`; Fact C is handled in the read path).
- Tests: `envelope` unit (parse matrix incl. magic-needs-16-bytes; `decode` round-trips raw + LZ4 across sizes spanning multiple chunks; **truncated trailing chunk ⇒ fewer logical bytes than `logical_len`, NOT zero-fill** (Fact C); malformed/short/oversized full-frame chunk ⇒ zero-fill within span; `logical_len > MAX`, codec=2, inconsistent table ⇒ empty/Invalid; partial read at and across chunk boundaries returns the exact logical prefix); `types` (legacy unchanged; envelope `total_len()` pre-read via eager header; full read decodes; truncated read trips the completeness assert; cache correctness); a checked-in envelope fixture (`test_data/block_with_envelope_blobs/`, generated once via docker) run through the existing `verification_*` matrix to prove the **unchanged verifier** accepts envelope blobs and a malformed-content (full-frame) envelope decodes to zero-fill (not rejected). All existing tests pass untouched.
- Determinism gate: a test asserting native==guest `decode` over a fixed corpus; `lz4_flex` version workspace-pinned.
- Not a breaking witness change; changelog notes read-path support + historical magic-collision caveat (~2⁻¹²⁸; such a v1 batch blob would have failed borsh anyway).

### PR3 — Submit path + config + e2e
- `config.rs` enum + field; `da_service/mod.rs` chunked encode-on-submit (+ magic-escape, fall-back-if-larger), `get_proofs_at` decode; metrics; documented `#compress_on_submit = "off"` in `examples/demo-rollup/configs/celestia_rollup_config.toml`; adapter README; changelog.
- Tests: config parse/default/roundtrip/schema; prepare-payload unit matrix (off-passthrough, magic-escape-even-when-off, incompressible-fallback, compressible-shrinks, proof-never-compressed); `get_proofs_at` parity; docker e2e (lz4 service: compressible + incompressible + magic-prefixed batches + a proof → read-back equals originals; extraction proof verifies for read/skip; posted < logical for the compressible one); mixed legacy+envelope namespace read by a compression-off (PR2-aware) service.

## Verification (each PR)
```bash
cargo fmt --all
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-celestia-adapter --features native
cargo check -p sov-celestia-adapter --no-default-features   # guest closure (decode must build)
```
Plus: the native==guest `decode` determinism test, and the explicit Fact-C test (truncated
trailing chunk yields `< total_len()` logical bytes so the completeness assert fires).
PR2/PR3 each need one fixture-generation session against the dockerized Celestia devnet.

## Disadvantages & tradeoffs (honest assessment)

Ordered roughly by how much they should weigh on the decision.

1. **`decode` becomes frozen, consensus-critical code — enlarged by chunking.** Once a blob is posted, every node and prover must forever decode it byte-identically — the exact chunk framing, the precise LZ4 output, and the exact full-frame-vs-prefix rule. This pins `lz4_flex` to one version and freezes the format; a "bugfix" to `decode` is a hard fork. Chunking grows this surface (framing + per-chunk logic + the logical↔compressed map) and the audit. Biggest long-term cost. (Inherent to any in-protocol compression; chunking adds to it.) Mitigations: workspace-pin `lz4_flex`; native==guest determinism CI; versioned format spec.

2. **Compute/DA asymmetry — a bounded decompression bomb.** A registered sequencer can post a few-KB compressed blob declaring a large `logical_len`; a full read forces the prover to decode it. **Bounded three ways:** the 16 MiB per-blob cap, gas charged on logical `total_len()` (`capabilities.rs:1046`, so the attacker pays ~in proportion), and the block capacity limiter measured in logical bytes (`:150`, so a block can't be packed with many max bombs). Chunking *improves* peak memory (one 64 KiB chunk at a time instead of 16 MiB) and lets a future reader stop early. Residual: decode is on the prover's critical path. Accept with caps + `off` default.

3. **Partial-read capability — delivered (the v3 "latent" item, now resolved).** Chunking makes envelope blobs prefix-decodable: a future STF/parser can authenticate and decode only the chunks covering a prefix and stop after a malformed region, without forcing the whole blob. Bought at the cost in #1. No current consumer (Fact A).

4. **Operational footgun → slashing (wants activation gating).** Turning `compress_on_submit` on before every node/prover is upgraded means an old node reads an envelope as raw → borsh fails → an **honest sequencer is slashed**. The `off` default helps, but a per-service config flag desyncs the instant one node lags or is misconfigured. **Recommendation (adversarial framing):** gate envelope *emission* on a protocol activation height / chain version, not just the local flag, so a blob is never emitted before every reader can decode it. (Distinct from Fact C, which is pure read-path code, not an activation matter.)

5. **Non-obvious invariants (bug-hiding surface).** Correct logical `total_len()` depends on construction having eager-advanced the header + chunk table into the accumulator at *every* construction site; plus the `OnceLock` cache to return `&[u8]` from a lazy decode; plus the chunk-aware logical↔compressed map; plus — most subtly — the Fact-C full-frame-vs-prefix split in `decode`. All reviewable, but more mechanism than "just decompress," and the place bugs hide. Covered by the explicit tests above; audit target.

6. **Rides on top of the pre-existing witness bloat.** Envelope blobs still carry their (now compressed, hence smaller) shares in the witness; compression neither fixes nor worsens the unread-share bloat. Pruning is a separate follow-up.

## Accepted risks / out of scope
- Historical magic-collision reinterpretation: ~2⁻¹²⁸, documented (outcome is slash either way).
- Old binaries can't decode envelope blobs: enabling `lz4` needs all nodes/provers on PR2+; `off` default + activation gating (disadvantage #4) make rollout operator-controlled and atomic.
- zstd reserved only (codec 2 → InvalidEnvelope/empty for current code; adding it later is a coordinated upgrade).
- **Witness unread-share bloat: pre-existing, unrelated to compression, not worsened. DECIDED: deferred to a separate follow-up** (Nikolai, 2026-06-15).
- Borsh trailing-byte / valid-prefix edge on the deserialize-*success* branch (the assert at `capabilities.rs:1063` only runs on failure): pre-existing for legacy blobs, not introduced by compression. Worth a one-line confirmation during PR2, not a blocker.
- Economics (bytes saved vs zk cycles): intentionally unmeasured; `off` default.
- `hash()` partial-authentication: pre-existing, unchanged.

## Open points
1. ~~Drop witness-pruning~~ — **RESOLVED: deferred to a follow-up.**
2. **Eager-advance the header + chunk table at construction** vs. a stored claim checked by the verifier. Picks eager-advance (no new field, no verifier change). See disadvantage #5.
3. ~~Single frame vs chunks~~ — **RESOLVED (2026-06-15): chunked**, to preserve partial reads. See "Decisions" and disadvantage #1 for the cost accepted.
4. **Activation gating** — recommend a protocol activation height/version for envelope emission (disadvantage #4), rather than relying on the per-service `off` flag alone. Open for Nikolai.

## Design intent: preserve partial reads — RESOLVED by chunking

Today's standard blob-storage STF path effectively reads blobs full-or-zero (Fact A): size
gates may skip a blob using `total_len()`, and accepted blobs are deserialized via
`full_data()` / `verified_data()`. That is a current usage pattern, not a limitation of the
DA/STF model — `BlobReaderTrait`, `CountedBufReader`, Celestia proof generation, and Celestia
verification are intentionally built around partial consumption.

The v3 single-frame envelope would have made envelope blobs inherently all-or-nothing,
foreclosing that capability without a format v2. **v4 resolves the tension by chunking from
the start:** legacy blobs keep partial-read behavior unchanged, and envelope blobs become
prefix-decodable at chunk granularity. The Fact-C decode contract guarantees a partial read
never fabricates logical bytes (a not-fully-authenticated chunk yields nothing), so partial
reads are both *available* and *safe*. The accepted cost is the larger consensus-critical /
audit surface (disadvantage #1); the benefit has no consumer yet (Fact A) and is purely
forward-looking.
