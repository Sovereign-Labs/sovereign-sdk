# Celestia Blob Compression — Actual Plan & Progress

Status: ACTIVE — implementation in progress
Scope: `crates/adapters/celestia` (self-contained; no public API changes)
Supersedes: `celestia-compression-claude.md`, `celestia-compression-claude-2.md`,
`celestia-compression-codex.md`, `celestia-compression-codex-2.md` (kept for history;
their claims were verified against the code and their gaps are corrected here).

## Progress

| PR | Content | Status |
|----|---------|--------|
| PR1 | Accessor split (`compressed_*`/`logical_*`) + `TotalLenMismatch` security fix | ✅ **Committed** (`43fb15f92`, branch `nikolai/celestia-compression-pr1-accessors`) |
| PR2 | Witness pruning: `BlobPayload` state enum + claim-only custom serde | ✅ Implemented & verified — **in working tree on `nikolai/celestia-compression-pr2-witness-pruning`, awaiting review/commit** |
| PR3 | Envelope format + read path + verifier Layer 2 (dark: nothing emits envelopes) | ⏳ Not started — gated on PR2 commit |
| PR4 | Submit path + `compress_on_submit` config + metrics + e2e | ⏳ Not started |

Process: each PR is implemented and verified in the working tree; **Nikolai reviews and
commits himself**. The next PR starts only after the previous one is committed.

Verification state of PR2 (as of 2026-06-12): 72/72 `sov-celestia-adapter` tests pass
(incl. docker integration suite and bincode+risc0 serde round-trips), clippy clean
(pre-existing warnings only), guest closure (`--no-default-features`) compiles, fmt
clean, breaking-change changelog entry added.

## Context & requirements

DA cost is a dominant rollup expense; batch blobs compress well. Compress batch blobs
at submission inside the adapter, decompress on read — transparent to all consumers
(`DaService` callers and `BlobReaderTrait` keep seeing logical bytes), configurable via
runtime `CelestiaConfig`, backward compatible with all previously posted blobs, and
preserving the ZK property that verifier work scales with bytes the STF consumed.

Scope decisions (settled with Nikolai):
- **LZ4 only in v1** (`lz4_flex`, pure Rust); the config enum and the wire format
  reserve zstd (config value + codec id 2) for a follow-up gated on cycle benchmarks.
- **Witness-pruning refactor included** as its own PR (PR2) — fixes a pre-existing
  witness-bloat issue and gives compression a clean state model.
- **No zk-cycle benchmark in this effort**; `compress_on_submit` defaults to `off`, so
  the code lands without exposure and the economics decision is operational.
- **Batch blobs only**; `send_proof` stays raw (read/verify paths still handle
  envelopes uniformly on both namespaces, since anyone can post anything).
- Multiple PRs, each independently reviewable.

## Facts verified against the code (and doc corrections)

All four prior design docs were checked claim-by-claim against the code. Confirmed
ground truth the design relies on:

- Submit funnel: `send_transaction`/`send_proof` → `submit_blob_to_namespace`
  (`da_service/mod.rs`), single `JsonBlob::new` wrap site; `blob_hash` = Celestia
  commitment of the posted bytes. Submitted blobs are v1 (signer present).
- v0 (signerless) blobs are skipped at extraction (`get_blobs_with_sender`) and proven
  as "skipped" → envelope logic only ever concerns v1 blobs.
- The verifier always proves **≥1 share per blob** (`.max(1)` in both
  `authenticate_blob_data` and `new_inclusion_proof`); signer and `sequence_length`
  are always read from the authenticated first share. The byte-compare loop checks the
  consumed prefix (accumulator) against share payloads.
- Proof generation reads the inner reader directly — share/continuity math is
  DA-physical by construction (made explicit by PR1's `compressed_*` accessors).
- STF read pattern is **always full reads**: native `full_data()`, guest <!-- Nikolai: < verify this -->
  `verified_data()` (`sov-blob-storage/src/capabilities.rs`,
  `data_for_deserialization`). All `total_len()` consumers want **logical** bytes:
  emergency-registration gate (`MAX_EMERGENCY_REGISTRATION_BLOB_SIZE = 1000`),
  per-byte deserialization gas charge, capacity limiter, `batch_selector`.
- **Load-bearing assert** (`capabilities.rs`, slashing path): after a failed batch
  deserialization, `assert_eq!(verified_data().len(), total_len())`. In the guest a
  violation is a panic ⇒ unprovable slot. Any design where a blob cannot deliver
  exactly `total_len()` bytes is broken.
- `advance`/`full_data` are `#[cfg(feature = "native")]`; the guest verifier must
  authenticate every byte before the STF runs (`stf/verifier.rs` runs DA verification
  first, then hands the same blobs to the STF).
- Guests (`examples/demo-rollup/provers/{risc0,sp1}/guest-celestia`) build the adapter
  without `native`; `lz4-sys` (via `celestia-client`) is not in the guest closure;
  `lz4_flex` is pure Rust and fits.

Corrections to the prior docs:
1. `celestia-compression-claude.md`'s 1-byte `0x00`-tag format is unsound — legacy
   blobs carry no tag, so the scheme is ambiguous. Magic-prefix auto-detect (the later
   docs) is the right mechanism.
2. The docs' "reject malformed envelopes" is **dangerous if implemented as a verifier
   error**: a malformed envelope is on DA and authenticated, so no witness could make
   the slot pass ⇒ chain halt. Malformed on-DA data must map to deterministic
   STF-visible semantics (see zero-fill below); verifier errors are reserved for
   witness lies.
3. `celestia-compression-claude-2.md` uses u16 chunk-length fields (caps chunks at
   65535 B); this plan uses u32 fields.
4. The docs' "eager-advance to authenticate the header" invariant is unnecessary:
   since ≥1 share is always proven, the header is parsed directly from the
   authenticated first-share payload in native and guest alike.
   **Mode is derived, never a witness claim.**
5. **Pre-existing soundness gap found during verification** (in no doc): the verifier
   never cross-checked the witness-recorded total length (prover-controlled) against
   the authenticated first-share `sequence_length`, while `total_len()` feeds gas and
   size gates in the STF — a lying prover could attest a divergent state transition.
   Fixed in PR1 (`BlobDataError::TotalLenMismatch` + inflated/deflated regression
   tests).

## Design

### Envelope format v1 (consensus-critical adapter constants, NOT config)

```
magic             [16] b"SOV_CELESTIA_CMP"   (ASCII, no 0x00 bytes — share zero-padding cannot fake it)
version           [1]  = 1
codec             [1]  0 = raw chunks, 1 = LZ4 block   (2 reserved for zstd)
flags             [2]  must be 0
logical_len       [4]  u32 LE, <= MAX_LOGICAL_BLOB_LEN
-- 24-byte fixed header (fits the first v1 share payload: 478 − 20 signer = 458 B), then chunks:
chunk_logical_len [4]  u32 LE — MUST be canonical: min(MAX_LOGICAL_CHUNK_LEN, logical_len − bytes_already_decoded)
chunk_encoded_len [4]  u32 LE — 0 < enc <= chunk_logical_len; enc == logical ⇒ chunk stored raw (even under codec=1)
chunk_payload     [chunk_encoded_len]
```

Constants: `MAX_LOGICAL_BLOB_LEN = 16 MiB`, `MAX_LOGICAL_CHUNK_LEN = 64 KiB`.

- Chunks are **independently decodable** — prefix-decodability is structural and
  codec-agnostic (adding zstd later changes nothing about the DoS bound).
- **Canonical chunking** (every chunk's logical length is forced) plus the per-chunk
  raw-store rule bound worst-case expansion to ≈1.0001× and eliminate
  degenerate-chunking amplification (tiny chunks would otherwise make compressed bytes
  ≫ logical bytes, under-charging gas relative to verifier work).
- `sum(chunk_logical_len) == logical_len` exactly; all chunks must lie within
  `sequence_length`.

### Mode detection — derived from authenticated bytes, never claimed

`mode = f(first_share_payload[..min(sequence_length, capacity)], sequence_length)`,
one shared function used by native extraction and the guest verifier:

- No 16-byte magic prefix (incl. `sequence_length < 16`) → **Legacy raw**: today's
  semantics byte-for-byte.
- Magic + valid header (`seq_len ≥ 24`, version 1, known codec, flags 0,
  `logical_len ≤ MAX`) → **Envelope**.
- Magic + invalid/truncated header → **InvalidEnvelope**: the blob is exposed as
  empty (`total_len() = 0`, no bytes). Decidable from share 1 alone (which is always
  proven); never a verifier error; never falls back to raw.

### STF-visible semantics — header-only + deterministic zero-fill (the key decision)

For envelope blobs:
- `total_len()` = header `logical_len`, **always** — independent of chunk validity.
  (If validity affected `total_len()`, the guest would have to decode every unread
  spam blob just to verify a validity bit, destroying the partial-read cost model.
  `batch_selector` reads `total_len()` of every blob, so this is observable even for
  blobs the STF never advances.)
- `verified_data()` = decompressed logical bytes. `advance(n)` decodes lazily:
  compressed side consumed in whole chunks, logical accumulator grows by exactly `n`
  (decoded remainder buffered in native-only state).
- **Any chunk-level failure** (non-canonical/oversized lengths, chunk overruns
  `sequence_length`, LZ4 decode error, wrong decoded size, stream exhausted before
  `logical_len`) ⇒ **all remaining logical bytes `[produced..logical_len)` are 0x00**,
  deterministically. On failure, native consumes compressed bytes up to the failure
  evidence (through the bad chunk header/payload, or to end of stream for truncation)
  so the guest can reproduce the failure from the authenticated compressed prefix.

Why zero-fill is forced (each alternative is concretely broken):
- Truncated reads ⇒ the `capabilities.rs` slashing-path assert panics ⇒ unprovable
  slot / crashed node.
- Verifier-error on malformed data ⇒ chain halt (correction #2).
- "Invalid ⇒ empty" with full validation ⇒ guest must decode all chunks of every
  unread envelope blob.

Consequences: reads stay total and lazy; a sequencer posting garbage fails borsh on
the zero-filled payload and is slashed via the existing path (the assert holds:
`verified == total`); unread spam blobs still cost exactly 1 proven share.

### Two-layer verifier (`authenticate_blob_data`)

- **Layer 1 (existing math, now explicitly over the compressed stream)**: share count
  from `compressed_verified_data().len()` `.max(1)`; byte-compare of the compressed
  accumulator against proven share payloads; NMT range proofs; signer check;
  occupancy from `sequence_length`.
- **Layer 2 (new)**: universal claim checks first —
  `compressed_total_len == sequence_length` (the PR1 fix),
  `compressed.len() ≤ compressed_total`, `logical.len() ≤ logical_total`. Then derive
  mode from the first share:
  - **Legacy** → `logical_total == compressed_total` and `logical == compressed`
    (structural with the `Raw` claim encoding).
  - **InvalidEnvelope** → `logical_total == 0` and `logical` empty.
  - **Envelope** → `logical_total == header.logical_len`; run the shared decoder over
    the compressed claim, comparing each emitted chunk (≤64 KiB at a time; never
    materialize the full output) against the logical claim. Terminal rules:
    `Complete`/`NeedMoreInput` with `logical.len() ≤ produced` ⇒ OK iff all compared
    bytes matched; `NeedMoreInput` with `logical.len() > produced` ⇒ **reject**
    (prover under-provided evidence); `Failed` ⇒ logical tail `[produced..]` must be
    all zeros.
  - The `NeedMoreInput` vs `Failed` distinction is load-bearing: a chunk extending
    past the *claim* but within `sequence_length` is under-provided evidence; a chunk
    extending past `sequence_length` is genuine truncation (zero-fill applies). The
    decoder therefore takes both the compressed prefix and `compressed_total_len`.

Invariant: **chunk-level problems are never verifier errors; witness lies always
are.** One shared `EnvelopeDecoder` drives both native `advance` and guest Layer 2 —
the native side must contain no chunk logic of its own.

### Witness claims (landed in PR2)

`BlobWithSender` = `{ payload: BlobPayload, range_in_namespace, sender, hash }`:

```rust
enum BlobPayload {
    #[cfg(feature = "native")]
    Native(NativeBlobPayload),      // owns CountedBufReader<BlobIterator>; advanceable
    Verified(BlobPayloadClaim),     // restored from a witness; read-only
}

enum BlobPayloadClaim {             // the ONLY thing serialized — variant order is wire format
    Raw { data: Vec<u8>, total_len: u64 },                                  // index 0 (PR2)
    Envelope { compressed: Vec<u8>, logical: Vec<u8>,
               compressed_total_len: u64, logical_total_len: u64 },         // index 1 (PR3, append-only)
}
```

- Custom `Serialize`/`Deserialize` via a mirror struct: serialization emits the pruned
  claim; deserialization always yields `Verified`. Unread shares never cross the
  witness boundary (unread ~54-share blob: < 600 B witness vs ~28 KiB before).
- The `Raw` vs `Envelope` claim encoding avoids duplicating bytes for legacy blobs.
  The encoding is *not* trusted: Layer 2 derives the mode from the authenticated share
  and rejects mismatched encodings.
- `Native` is feature-gated — a guest build cannot hold the share source.
- `PartialEq` is defined over the claim view (serialization is intentionally lossy).
- `advance` on `Verified` panics if it would read into the pruned remainder; no-ops
  otherwise (so `full_data()` on a fully-read restored blob is fine).

### Submit path & config (PR4)

- `CelestiaConfig.compress_on_submit: CompressOnSubmit` —
  `#[serde(rename_all = "lowercase")] enum { #[default] Off, Lz4, Zstd }` (+
  JsonSchema). `CelestiaService` construction rejects `Zstd` ("reserved, not
  implemented").
- `send_transaction` (batch namespace): `Off` ⇒ passthrough; `Lz4` ⇒ encode (64 KiB
  canonical chunks; per-chunk raw-store when incompressible); if the envelope is not
  strictly smaller than the raw payload ⇒ passthrough. **Always** wrap payloads that
  start with the magic in a codec=0 raw envelope, even when `Off` — detection must
  stay unambiguous.
- `send_proof`: never compresses; magic-escape only.
- `get_proofs_at_inner`: unwrap envelopes with the same decoder so the native
  proof-fetch path matches the guest's view of proof-namespace blobs.
- Metrics: logical vs posted bytes, saved basis points (integer), outcome tag.
- Capacity/fee note: compression is a cost optimization only — batch sizing
  (`max_batch_size_bytes`) stays in logical bytes; Celestia fees are paid on posted
  (compressed) bytes automatically.

## PR breakdown

### PR1 — Accessor split + `total_len` authentication fix ✅ (committed `43fb15f92`)
- `compressed_*`/`logical_*` accessors on `BlobWithSender`; verifier/proof-gen/tests
  migrated; `BlobReaderTrait` docs clarified ("logical bytes; adapters may store a
  different physical representation").
- Security fix: `compressed_total_len == sequence_length` in
  `authenticate_blob_data` (`BlobDataError::TotalLenMismatch`), with inflated and
  deflated forged-witness regression tests (both passed verification before the fix).

### PR2 — Witness pruning ✅ (in working tree, awaiting review/commit)
- `BlobPayload`/`NativeBlobPayload`/`BlobPayloadClaim::Raw` + custom serde + manual
  `PartialEq` + `Verified` advance guard, as described above.
- PR1's forge test path updated: `["blob"]["inner"]["sequence_len"]` →
  `["payload"]["Raw"]["total_len"]`.
- New tests: `witness_excludes_unread_shares`, `witness_roundtrip_preserves_claims`
  (bincode + risc0, read depths 0/1/half/full),
  `verified_blob_advance_is_noop_when_fully_read`,
  `verified_blob_panics_on_advance_into_pruned_remainder`.
- Changelog: **Breaking Change** — witness format; nodes and provers upgrade together.

### PR3 — Envelope module + read path + verifier Layer 2 (dark) ⏳
- New `crates/adapters/celestia/src/envelope.rs` (not cfg-gated; guest needs it):
  constants, `Codec`, `EnvelopeHeader`, `BlobFormat`, `parse_blob_format`,
  `EnvelopeDecoder` (`next_step() -> Chunk|Complete|NeedMoreInput|Failed`,
  `evidence_end()`), `verify_blob_claims` (Layer 2),
  `#[cfg(feature = "native")] encode_envelope`, `starts_with_magic`.
- Workspace dep: `lz4_flex = { version = "0.11", default-features = false,
  features = ["std", "safe-encode", "safe-decode"] }` (pure Rust, no build.rs; same
  version native+guest via workspace ⇒ decode determinism).
- `types/mod.rs`: `NativeBlobPayload` gains `format` + lazy decode state (`pending`
  buffer, `failed` flag); `get_blobs_with_sender` parses the format from the first
  share before building the iterator; `total_len()` returns logical; `advance(n)`
  only drives `next_step()`; `BlobPayloadClaim::Envelope` appended (variant 1 —
  never reorder).
- `verifier/mod.rs`: hoist first-share/`sequence_length` extraction; append Layer 2.
  `types/error.rs`: `NonMatchingLogicalData`, `LogicalClaimBeyondDecodedData`,
  `NonZeroFillTail`, `InvalidClaimLengths`.
- New checked-in fixture `test_data/block_with_envelope_blobs/` (one dockerized
  devnet generation session, existing `update_test_data` pattern): valid lz4
  multi-chunk/single-chunk, codec=0 envelope, `logical_len == 0`, magic+truncated
  header, bad version / codec=2 / nonzero flags / oversized `logical_len`,
  non-canonical chunk, `enc > logical`, lz4-garbage chunk, truncated final chunk,
  plus a legacy blob in the same namespace.
- Tests: envelope unit tests (header-parse matrix incl. the zero-padding rule;
  proptest encode→decode identity over 0..300 KiB compressible and random payloads;
  canonical-chunking enforcement; `NeedMoreInput` vs `Failed` on identical bytes with
  different `compressed_total_len`; per-error evidence offsets); types tests (logical
  `total_len`; partial reads at 1 / 64Ki−1 / 64Ki / 64Ki+1; zero-fill per malformed
  fixture — `full_data()` = prefix+zeros, no panic; invalid header ⇒ empty);
  verifier tests (fixture through the full read-pattern matrix incl. reads into
  zero-fill regions; **witness-lie matrix** — each mutated claim must produce its
  named error; mismatched claim encodings rejected).
- Changelog: read-path envelope support; additive witness variant (breaking); the
  historical-collision caveat (a pre-existing v1 batch blob starting with the 16-byte
  magic — probability ≈2⁻¹²⁸, and such a blob would have failed borsh anyway —
  re-derives differently under the new adapter).

### PR4 — Submit path + config + e2e ⏳
- `config.rs` enum + field; `prepare_batch_payload`/`prepare_proof_payload`;
  `get_proofs_at` unwrap; metrics; `examples/demo-rollup/configs/
  celestia_rollup_config.toml` documented `#compress_on_submit = "off"`; adapter
  README format spec; changelog.
- Tests: config parse/default/roundtrip/schema; prepare-payload unit matrix
  (off-passthrough, magic-escape-even-when-off, incompressible-fallback,
  compressible-shrinks, proof-never-compressed); decoder parity for `get_proofs_at`;
  docker e2e (lz4 service: compressible + incompressible + magic-prefixed batches +
  a proof → read-back equals originals; extraction proof verifies for
  no-read/partial/full; posted < logical for the compressible batch); mixed
  legacy+envelope namespace read by a compression-off service.

## Verification (each PR)

```bash
cargo fmt --all
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-celestia-adapter --features native
cargo check -p sov-celestia-adapter --no-default-features   # guest closure
```

PR3/PR4 additionally need one fixture-generation session against the dockerized
Celestia devnet. PR4 runs the verify-example-configs flow. Each PR adds a CHANGELOG.md
entry (replace `#PR_NUMBER` placeholders when PRs are opened).

## Accepted risks / out of scope

- Historical magic-collision reinterpretation: ≈2⁻¹²⁸, documented in release notes.
- Old binaries cannot read new envelope blobs: enabling `lz4` requires all
  nodes/provers upgraded first; `off` default makes rollout operator-controlled.
- zstd: reserved only. Old verifiers see codec-2 blobs as InvalidEnvelope (empty);
  adding zstd later is a coordinated upgrade like any format revision.
- Economics (bytes saved vs zk cycles) intentionally unmeasured in this effort.
- `hash()` partial-authentication concern from the prior docs: pre-existing,
  unchanged by compression, out of scope.
