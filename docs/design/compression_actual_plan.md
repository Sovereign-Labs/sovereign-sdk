# Celestia Blob Compression — Actual Plan & Progress (v4)

Status: ACTIVE — reworked from v3 after a security + effort review (2026-06-15). Two
changes from v3: (1) the envelope is **chunked from the start** (not a single LZ4 frame),
to preserve the SDK's partial-read capability; (2) the `decode` contract is **corrected**
— it must never emit logical bytes for un-authenticated input (the v3 "zero-fill to
`logical_len` on short/error" rule was unsafe under an adversarial prover; see Fact C).
Scope: `crates/adapters/celestia` (self-contained; no public API changes). The existing
compressed-byte verifier checks remain the foundation, but PR2 adds envelope-specific
metadata/read-cursor authentication where those checks alone do not authenticate
STF-observable logical fields.
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

Because the accumulator is byte-compared against `share.payload()`, and shares hold the **compressed/on-DA** bytes, **the accumulator is compressed bytes**, authenticated as-is. Logical bytes cannot be tied to shares — they are **decompressed** from the authenticated accumulator. The witness must never carry trusted logical bytes. It may carry a logical read cursor, but PR2 must authenticate that cursor against the decoded output available from the authenticated compressed prefix. Proof generation uses `compressed_*` exclusively (`proofs.rs:175,188`).

### Fact C — the verifier+STF interlock that stops a truncating prover on failure paths

Today, on the standard deserialization-failure path, a malicious prover cannot slash an honest sequencer by truncating the witness: bytes are authenticated (it cannot corrupt them), and *under-reading* is caught by an interlock:
- the verifier pins `compressed_total_len() == sequence_length` (`verifier/mod.rs:451`) — note `total_len() = inner.remaining() + accumulator.len()`, so the prover cannot lie about the blob's full length; and
- the STF asserts `verified_data().len() == total_len()` (`capabilities.rs:1063`, *"…some data was not provided. The prover might be malicious"*).

A prover who authenticates only a 500-of-1000-byte prefix (legal at the compressed-prefix proof layer — the verifier proves only `shares_needed(500)` shares against the real prefix) trips the assert (`500 != 1000`) if deserialization fails → guest panic → no valid proof.

**Compression must not break this interlock or the size-gate paths that run before deserialization.** The v3 `decode` rule ("force output to exactly `logical_len`, zero-fill on any short/error") *does* break it: logical `total_len()` becomes a header value decoupled from the authenticated compressed length, and `verified_data()` becomes `decode(prefix)` zero-filled back to `logical_len`, so the assert degenerates to `logical_len == logical_len` (always true). A malicious prover could then authenticate a prefix → zero-fill → borsh fails on garbage → **honest sequencer slashed with a valid proof.** The corrected contract below restores the failure-path interlock and adds verifier/read-path checks for STF-observable metadata, so missing envelope evidence cannot silently turn into an empty or skipped blob. The deserialize-success prefix/trailing-byte edge remains a pre-existing risk called out below and must be confirmed during PR2.

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
need storing, and the table for a 16 MiB blob is ≤256×4 B ≈ 1 KiB. The header +
chunk table are authenticated as normal consumed compressed bytes: they are included in
the accumulator and byte-compared against shares. This is bounded metadata overhead, not
free; large envelopes may require a few shares even for a logical zero-read.

### Mode detection — derived from the (authenticated) accumulator, never claimed

`mode = f(authenticated first-share payload, accumulator_prefix, sequence_length)`, one shared pure function. The verifier must derive the same mode from authenticated share bytes before accepting STF-observable envelope fields:
- no 16-byte magic prefix in the authenticated first-share payload → **Legacy** (today's behavior, byte-for-byte).
- magic + complete, valid header (version 1, known codec, flags 0, `logical_len ≤ MAX`) + complete chunk table consistent with `logical_len` → **Envelope**.
- magic + complete but invalid/oversized header or complete but inconsistent table → **InvalidAuthenticatedEnvelope** → behaves as an empty blob (`total_len()=0`, no data). This is authenticated malformed DA content, not a witness shortcut.
- magic present but the accumulator does not include the complete header/table needed to classify the blob, or does not include the compressed chunks needed for the claimed logical cursor → **InsufficientEnvelopeEvidence** → witness/proof error before STF execution. It must never be treated as `InvalidAuthenticatedEnvelope` or as a valid empty blob.

Chunk-table validity uses checked arithmetic throughout: derive the table byte length only after `logical_len <= MAX`, cap each encoded length, reject offsets outside `sequence_length`, require `sum(encoded_lengths) == compressed_total_len - metadata_len` for full-frame validation, and require `codec=0` chunk lengths to equal their logical chunk span exactly.

### Deterministic, total, chunk-aware decode (the corrected logic — the only new logic)

`decode(accumulator, compressed_total_len, logical_verified_len) -> DecodeOutput`, identical native and guest. It never rejects authenticated DA bytes as a verifier error, but it must distinguish authenticated malformed content from insufficient witness evidence. Two clauses, and the split between them is what restores Fact C's interlock:

- **Full frame authenticated** (`accumulator.len() == compressed_total_len`, i.e. every
  chunk's compressed bytes are present): decode is **total** and returns **exactly
  `logical_len` bytes**. Each chunk is LZ4-decoded to its declared logical span; any chunk
  whose block is malformed / short / over-long is **zero-filled within its span**.
  Header/table inconsistency is handled earlier as `InvalidAuthenticatedEnvelope`.
  (Garbage *content* on DA ⇒ deterministic logical bytes
  ⇒ borsh fails ⇒ existing slash path. Safe because the bytes are authenticated real on-DA
  bytes — a prover cannot manufacture this for an honest sequencer.)
- **Only a prefix authenticated** (`accumulator.len() < compressed_total_len`): decode the
  maximal run of chunks fully contained in the accumulator, and no more — a
  partial/truncated trailing chunk contributes **zero** logical bytes, never zero-fill.
  `logical_verified_len` must be `<= available_decoded_len`; otherwise the verifier rejects
  `InsufficientEnvelopeEvidence` before the STF. `verified_data()` returns exactly the first
  `logical_verified_len` bytes, so `advance(1)` exposes one logical byte even if the
  compressed accumulator rounded up to a full chunk. For a real truncation on today's full
  read path this yields `< logical_len` ⇒ the STF completeness assert (`capabilities.rs:1063`)
  fires on deserialization failure. For a legitimate future partial read it is exactly the
  requested logical prefix, nothing fabricated.

- Legacy → the accumulator unchanged (returned by reference; no copy).
- InvalidAuthenticatedEnvelope → empty.
- InsufficientEnvelopeEvidence → verifier/read-path failure before STF execution, never empty.

The prover cannot cheat: the guest decodes the same authenticated accumulator the prover's
native execution consumed, so any divergence yields a different state root than committed
(invalid proof). The `MAX_LOGICAL_BLOB_LEN` and per-chunk caps live in this shared `decode`,
applied identically native and guest.

### `BlobWithSender` — one type, minimal change

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]   // public API unchanged; witness compatibility handled below
pub struct BlobWithSender {
    blob: CountedBufReader<BlobIterator>,   // the COMPRESSED stream (accumulator = consumed on-DA bytes)
    logical_verified_len: usize,            // logical prefix exposed through BlobReaderTrait
    range_in_namespace: Range<usize>,
    sender: CelestiaAddress,
    hash: HexHash,
    #[serde(skip)]
    decoded: Option<DecodeCache>,           // keyed by (compressed_len, logical_verified_len)
}
```
- **Public API shape unchanged** — `BlobReaderTrait` and `DaService` callers are unchanged. The witness/internal serialization may gain the logical cursor; if so, PR2 must treat this as a witness-format change or provide explicit serde compatibility (`serde(default)`/custom serde) and document the rollout.
- `compressed_verified_data()` = `blob.accumulator()`, `compressed_total_len()` = `blob.total_len()` — **unchanged**; verifier and proof-gen keep using these.
- `verified_data()` (logical): Legacy → `blob.accumulator()` (zero-overhead). Envelope/Invalid → decode/cache the authenticated compressed prefix and return exactly `logical_verified_len` logical bytes.
- `total_len()` (logical): Legacy → `compressed_total_len()`. Envelope → `logical_len` parsed from the authenticated header. Invalid → 0.
- `advance(n)` (native): Legacy → `blob.advance(n)`. Envelope → increment `logical_verified_len` by exactly `n` (clamped to `logical_total_len`) and advance the *compressed* accumulator to cover the chunks spanning that logical prefix (`chunk = logical_offset / CHUNK_LOGICAL_LEN`; consume those chunks' compressed bytes per the table). The only caller today is `full_data()` (n = the rest), which consumes the whole frame; the chunk machinery exists so a future partial reader can stop after a malformed prefix.
- The decode cache must be invalidated or keyed whenever either `compressed_verified_data().len()` or `logical_verified_len` changes. A bare `OnceLock<Vec<u8>>` is invalid because `verified_data()` may be called before a later `advance()`.
- **Construction** (`get_blobs_with_sender`, native, `types/mod.rs:200-234`): peek the first share for the magic; if Envelope, eagerly `advance` the **header + chunk table** so logical `total_len()` and the chunk map are available before any read (the size gates call `total_len()` first). These metadata bytes are authenticated like any other consumed bytes; expect bounded small proof overhead rather than zero extra shares.
- Manual `PartialEq` over `{blob, logical_verified_len, range, sender, hash}` (ignore the decoded cache).

### Verifier additions

Layer 1 stays exactly the existing compressed-byte verifier behavior: authenticate the
compressed accumulator against shares, check `compressed_total_len == sequence_length`
(PR1, `verifier/mod.rs:451`), and keep proof generation over `compressed_*`.

PR2 adds an envelope Layer 2 before handing blobs to the STF:
- derive mode from the authenticated first share and the authenticated compressed
  accumulator, not from witness claims;
- require complete authenticated header + chunk table before accepting any envelope
  `total_len()` or chunk map;
- validate the logical read cursor (`logical_verified_len <= decoded_available_len`) and
  reject `InsufficientEnvelopeEvidence`;
- allow `InvalidAuthenticatedEnvelope`/zero-fill semantics only for malformed bytes that
  are themselves authenticated.

This keeps verifier errors reserved for witness/proof lies and deterministic decode results
reserved for authenticated DA content.

### Submit path & config (PR3)

- `CelestiaConfig.compress_on_submit: CompressOnSubmit` — `#[serde(rename_all="lowercase")] enum { #[default] Off, Lz4, Zstd }` (+ JsonSchema, following the `TxPriority` pattern in `config.rs`). Construction rejects `Zstd` ("reserved"). If activation gating is adopted (disadvantage #4), this pairs with a protocol activation height rather than standing alone.
- `send_transaction` (batch namespace): `Lz4` → `encode` (split logical into `CHUNK_LOGICAL_LEN` chunks, compress each, build header + table + chunks; fall back to passthrough if the envelope is not strictly smaller). **Always** wrap magic-prefixed payloads in a codec=0 envelope even when `Off` (keeps legacy detection unambiguous). `send_proof` → magic-escape only. `get_proofs_at_inner` → `decode` for transparency. Hash/commitment math untouched (`da_service/mod.rs:116`, commitment is over posted bytes).
- Metrics (logical vs posted bytes, saved basis points, outcome).

## PR breakdown

### PR1 — Accessor split + `total_len` auth ✅ committed (`43fb15f92`)

### PR2 — Chunked envelope read path (dark)
- New `crates/adapters/celestia/src/envelope.rs` (not cfg-gated; guest needs `decode`): constants (`MAGIC`, `MAX_LOGICAL_BLOB_LEN`, `CHUNK_LOGICAL_LEN`), `Codec`, header + chunk-table parse, `classify`, the two-clause `decode` above, `#[cfg(feature="native")] encode`, `starts_with_magic`. The chunk map + the full-frame-vs-prefix split is the bulk and the riskiest code — audit target #1.
- Dep: workspace `lz4_flex = { version = "0.11", default-features = false, features = ["std","safe-decode","safe-encode"] }`, **non-optional** (guest decode needs it; pure Rust; same version native+guest ⇒ decode determinism). Verify it compiles for both risc0 and sp1 guest targets (guest has `std`).
- `types/mod.rs`: add `logical_verified_len` plus an invalidatable/keyed decoded cache + manual `PartialEq`; make `verified_data()`/`total_len()`/`advance()` chunk-aware; `get_blobs_with_sender` peeks the magic and eager-advances the header + chunk table for envelopes.
- `verifier/mod.rs`: keep Layer 1 compressed checks, then add envelope Layer 2 for mode derivation, metadata completeness, chunk-table validity, and logical cursor authentication. `verifier/proofs.rs` remains over `compressed_*`.
- Tests: `envelope` unit (parse matrix incl. magic-needs-16-bytes; `decode` round-trips raw + LZ4 across sizes spanning multiple chunks; **truncated trailing chunk ⇒ fewer logical bytes than `logical_len`, NOT zero-fill** (Fact C); malformed/short/oversized full-frame chunk ⇒ zero-fill within span; `logical_len > MAX`, codec=2, inconsistent authenticated table ⇒ InvalidAuthenticatedEnvelope/empty; incomplete header/table/chunk evidence ⇒ InsufficientEnvelopeEvidence/reject; partial read at and across chunk boundaries returns the exact logical prefix); `types` (legacy unchanged; envelope `total_len()` pre-read via eager metadata; full read decodes; partial `advance(1)` exposes exactly one byte; cache invalidates after later `advance`; truncated read trips the completeness assert on failure paths); a checked-in envelope fixture (`test_data/block_with_envelope_blobs/`, generated once via docker) run through the verification matrix to prove envelope blobs verify, malformed authenticated content decodes deterministically, and insufficient evidence is rejected. All existing tests pass untouched.
- Determinism gate: a test asserting native==guest `decode` over a fixed corpus; `lz4_flex` version workspace-pinned.
- Witness compatibility: if `logical_verified_len` is serialized, PR2 is a witness-format change unless custom/defaulted serde preserves legacy decoding. Changelog notes read-path support + any witness compatibility impact + historical magic-collision caveat (~2⁻¹²⁸; such a v1 batch blob would have failed borsh anyway).

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

5. **Non-obvious invariants (bug-hiding surface).** Correct logical `total_len()` depends on construction having eager-advanced the header + chunk table into the accumulator at *every* construction site; plus an invalidatable/keyed cache to return `&[u8]` from a lazy decode; plus the logical cursor; plus the chunk-aware logical↔compressed map; plus — most subtly — the Fact-C full-frame-vs-prefix split in `decode`. All reviewable, but more mechanism than "just decompress," and the place bugs hide. Covered by the explicit tests above; audit target.

6. **Rides on top of the pre-existing witness bloat.** Envelope blobs still carry their (now compressed, hence smaller) shares in the witness; compression neither fixes nor worsens the unread-share bloat. Pruning is a separate follow-up.

## Accepted risks / out of scope
- Historical magic-collision reinterpretation: ~2⁻¹²⁸, documented (outcome is slash either way).
- Old binaries can't decode envelope blobs: enabling `lz4` needs all nodes/provers on PR2+; `off` default + activation gating (disadvantage #4) make rollout operator-controlled and atomic.
- zstd reserved only (codec 2 → InvalidAuthenticatedEnvelope/empty for current code; adding it later is a coordinated upgrade).
- **Witness unread-share bloat: pre-existing, unrelated to compression, not worsened. DECIDED: deferred to a separate follow-up** (Nikolai, 2026-06-15).
- Borsh trailing-byte / valid-prefix edge on the deserialize-*success* branch (the assert at `capabilities.rs:1063` only runs on failure): pre-existing for legacy blobs, not introduced by compression. Worth a one-line confirmation during PR2, not a blocker.
- Economics (bytes saved vs zk cycles): intentionally unmeasured; `off` default.
- `hash()` partial-authentication: pre-existing, unchanged.

## Open points
1. ~~Drop witness-pruning~~ — **RESOLVED: deferred to a follow-up.**
2. **Eager-advance the header + chunk table at construction** vs. a stored claim checked by the verifier. Picks eager-advance plus verifier validation of the authenticated metadata; this is bounded metadata proof overhead, not zero-cost. See disadvantage #5.
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
