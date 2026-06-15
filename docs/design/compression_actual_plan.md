# Celestia Blob Compression — Actual Plan & Progress (v3)

Status: ACTIVE — design reworked to the minimal, one-type, verifier-unchanged form below.
Scope: `crates/adapters/celestia` (self-contained; no public API changes).
Supersedes: `celestia-compression-{claude,claude-2,codex,codex-2}.md` (kept for history).

## Progress

| PR | Content | Status |
|----|---------|--------|
| PR1 | Accessor split (`compressed_*`/`logical_*`) + `TotalLenMismatch` security fix | ✅ **Committed** (`43fb15f92`) |
| PR2 | Envelope read path: `envelope.rs` + decode-on-read on `BlobWithSender` (dark; nothing emits envelopes) | ⏳ Not started |
| PR3 | Submit path + `compress_on_submit` config + metrics + e2e | ⏳ Not started |

**Dropped:** the earlier "witness-pruning via a `Native`/`Verified` enum + custom serde" PR. It was over-engineered, and the `Verified` label implied trust that does not exist — witness contents are attacker-controlled until authenticated. Witness-pruning is now **out of scope** (the pre-existing unread-share bloat is unrelated to compression and not worsened by it).

Process: each PR is implemented + verified in the working tree; **Nikolai reviews and commits himself**. The next PR starts only after the previous one is committed.

## Two facts that shape (and shrink) the design

### Fact A — reads are full-or-zero, never a partial prefix (verified in code)

A DA blob is read by the STF in exactly one of two ways:
- **Full read** — `data_for_deserialization(blob)` returns `full_data()` (native) / `verified_data()` (guest), then borsh `try_from_slice` deserializes the whole slice in one call (`sov-blob-storage/src/capabilities.rs`).
- **Zero data bytes read** — the blob is skipped by a `total_len()`-only gate *before* any payload byte is read: capacity limiter (`capabilities.rs:150`), emergency-registration size gate (`:230`, `MAX_EMERGENCY_REGISTRATION_BLOB_SIZE = 1000`), gas pre-charge (`:1046`), `batch_selector`.

There is **no path that reads a non-trivial prefix and abandons the rest.** The completeness assert (`capabilities.rs:1063`, `verified_data().len() == total_len()`) fires only after a *failed* deserialization — i.e. after a full read. The "8 MB blob with a problem at the start → skipped" case is the **zero-read** path: it is rejected by a `total_len()` gate without reading payload, so the verifier authenticates just 1 share (via `.max(1)`).

> An earlier version of this doc claimed "STF read pattern is **always full reads**." That was wrong. The correct statement is **full-or-zero**.

**Implication: no chunking.** A chunked envelope + partial-decode state machine only ever bought *partial-read* cost bounds. With full-or-zero reads, the envelope is a single compressed frame: full read → decode it all; skip → decode nothing.

### Fact B — the verifier trusts nothing from the witness

Trust comes only from the block header and the Celestia protocol, never from prover-serialized witness data:
1. `block_header` is the trusted public input. `validate_dah()` checks `dah.hash() == header.data_hash` → binds row roots to the header.
2. `proof.verify_range(row_root, shares.map(s.data()), namespace)` (nmt-rs) authenticates **share bytes**.
3. `sequence_length` / `signer` / `payload()` from the proven first share are trusted.
4. Witness fields are authenticated **against** those shares: the consumed-bytes accumulator is **byte-compared vs `share.payload()`**; `total_len` (compressed) `== sequence_length` (PR1); `sender == recovered signer`. `range_in_namespace` / `hash` are not read by the guest verifier.
5. Shares proven `= shares_needed_for_bytes_with_signer(accumulator.len()).max(1)`, matched exactly → work scales with bytes consumed.

Because the accumulator is byte-compared against `share.payload()`, and shares hold the **compressed/on-DA** bytes, **the accumulator is compressed bytes**, authenticated as-is. Logical bytes cannot be tied to shares — they are **decompressed** from the authenticated accumulator (a deterministic, total, trusted function). The witness never carries logical bytes and the verifier never "compares decoded vs claimed."

## Design

### Envelope format v1 (consensus-critical adapter constants, NOT config)

```
magic        [16] b"SOV_CELESTIA_CMP"   (ASCII, no 0x00 — share zero-padding can't fake it)
version      [1]  = 1
codec        [1]  0 = raw, 1 = LZ4 block   (2 reserved for zstd)
flags        [2]  must be 0
logical_len  [4]  u32 LE, <= MAX_LOGICAL_BLOB_LEN
-- 24-byte fixed header (fits the first v1 share payload), then a single compressed payload:
payload      [..] codec=0: raw bytes (== logical); codec=1: one LZ4 block decoding to logical_len
```
Constant: `MAX_LOGICAL_BLOB_LEN = 16 MiB` (decompression-bomb cap). No chunks.

### Mode detection — derived from the (authenticated) accumulator, never claimed

`mode = f(accumulator_prefix, sequence_length)`, one shared pure function:
- no 16-byte magic prefix (incl. prefix `< 16`) → **Legacy** (today's behavior, byte-for-byte).
- magic + valid 24-byte header (version 1, known codec, flags 0, `logical_len ≤ MAX`) → **Envelope**.
- magic + invalid/oversized header → **InvalidEnvelope** → behaves as an empty blob (`total_len()=0`, no data).

### Deterministic, total decode (the only new logic)

`decode(accumulator) -> Vec<u8>`, identical native and guest, never fails:
- Legacy → the accumulator unchanged (returned by reference; no copy).
- Envelope: parse header. Header-only accumulator (zero-read) → empty. Otherwise decompress `payload` (codec=0 → raw bytes; codec=1 → LZ4), forcing the result to exactly `logical_len` bytes — **zero-fill** on any decode error / short / over-long output. `logical_len` capped at `MAX_LOGICAL_BLOB_LEN`.
- InvalidEnvelope → empty.

Garbage on DA decodes deterministically (zero-fill) → borsh fails → existing slash path; it is **never** a verifier error (rejecting on-DA bytes would halt the chain). The prover cannot cheat: the guest decodes the same authenticated accumulator the prover's native execution consumed, so any divergence yields a different state root than committed (invalid proof).

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
- `verified_data()` (logical): Legacy → `blob.accumulator()` (zero-overhead). Envelope/Invalid → `decoded.get_or_init(|| decode(blob.accumulator()))`.
- `total_len()` (logical): Legacy → `compressed_total_len()`. Envelope → `logical_len` parsed from `blob.accumulator()[..24]`. Invalid → 0.
- `advance(n)` (native): Legacy → `blob.advance(n)`. Envelope → consume all remaining compressed (single frame is all-or-nothing; the only caller is `full_data()`, n = the rest).
- **Construction** (`get_blobs_with_sender`, native): peek the first share for the magic; if Envelope, eagerly `advance(24)` so the header is in the accumulator. This makes logical `total_len()` available before any read (the size gates call it first) **and authenticated for free** — the header bytes are in the accumulator, which the verifier already byte-compares against share 0. No new field, no new verifier check, 0 extra shares (24 B ⊂ first share; `.max(1)` already proves 1).
- Manual `PartialEq` over `{blob, range, sender, hash}` (ignore the `decoded` cache).

### Why the verifier is unchanged

It already authenticates the compressed accumulator against shares and checks `compressed_total_len == sequence_length` (PR1). Mode, `logical_len`, and logical bytes are **deterministic functions of the authenticated accumulator**, computed in the read path after verification. The `MAX_LOGICAL_BLOB_LEN` cap lives in the shared `decode`, applied identically native and guest. (Audit-4's "no panic / typed errors / fail-closed" applies to header parsing — `decode` is bounds-checked and total.)

### Submit path & config (PR3)

- `CelestiaConfig.compress_on_submit: CompressOnSubmit` — `#[serde(rename_all="lowercase")] enum { #[default] Off, Lz4, Zstd }` (+ JsonSchema). Construction rejects `Zstd` ("reserved").
- `send_transaction` (batch namespace): `Lz4` → `encode` (fall back to passthrough if the envelope is not strictly smaller). **Always** wrap magic-prefixed payloads in a codec=0 envelope even when `Off` (keeps legacy detection unambiguous). `send_proof` → magic-escape only. `get_proofs_at_inner` → `decode` for transparency. Hash/commitment math untouched.
- Metrics (logical vs posted bytes, saved basis points, outcome).

## PR breakdown

### PR1 — Accessor split + `total_len` auth ✅ committed (`43fb15f92`)

### PR2 — Envelope read path (dark)
- New `crates/adapters/celestia/src/envelope.rs` (not cfg-gated; guest needs `decode`): constants, `Codec`, `classify`/`parse_header`, `decode`, `#[cfg(feature="native")] encode`, `starts_with_magic`.
- Dep: workspace `lz4_flex = { version = "0.11", default-features = false, features = ["std","safe-decode","safe-encode"] }` (pure Rust; same version native+guest ⇒ decode determinism).
- `types/mod.rs`: add `#[serde(skip)] decoded` cache + manual `PartialEq`; make `verified_data()`/`total_len()`/`advance()` mode-aware; `get_blobs_with_sender` peeks the magic and eager-advances the 24-byte header for envelopes.
- `verifier/mod.rs`, `verifier/proofs.rs`: **unchanged** (already use `compressed_*`).
- Tests: `envelope` unit (parse matrix incl. magic-needs-16-bytes; `decode` round-trips raw + LZ4 over 0..300 KiB; malformed/short/oversized → zero-fill; `logical_len > MAX` and codec=2 → empty); `types` (legacy unchanged; envelope `total_len()` pre-read via eager header; full read decodes; zero-read empty; cache correctness); a checked-in envelope fixture (`test_data/block_with_envelope_blobs/`, generated once via docker) run through the existing `verification_*` matrix to prove the **unchanged verifier** accepts envelope blobs and a malformed-content envelope decodes to zero-fill (not rejected). All existing tests pass untouched.
- Not a breaking witness change; changelog notes read-path support + historical magic-collision caveat (~2⁻¹²⁸; such a v1 batch blob would have failed borsh anyway).

### PR3 — Submit path + config + e2e
- `config.rs` enum + field; `da_service/mod.rs` encode-on-submit (+ magic-escape, fall-back-if-larger), `get_proofs_at` decode; metrics; documented `#compress_on_submit = "off"` in `examples/demo-rollup/configs/celestia_rollup_config.toml`; adapter README; changelog.
- Tests: config parse/default/roundtrip/schema; prepare-payload unit matrix (off-passthrough, magic-escape-even-when-off, incompressible-fallback, compressible-shrinks, proof-never-compressed); `get_proofs_at` parity; docker e2e (lz4 service: compressible + incompressible + magic-prefixed batches + a proof → read-back equals originals; extraction proof verifies for read/skip; posted < logical for the compressible one); mixed legacy+envelope namespace read by a compression-off (PR2-aware) service.

## Verification (each PR)
```bash
cargo fmt --all
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-celestia-adapter --features native
cargo check -p sov-celestia-adapter --no-default-features   # guest closure
```
PR2/PR3 need one fixture-generation session against the dockerized Celestia devnet.

## Disadvantages & tradeoffs (honest assessment)

Ordered roughly by how much they should weigh on the decision.

1. **`decode` becomes frozen, consensus-critical code.** Once a blob is posted, every node and prover must forever decode it byte-identically — including the exact zero-fill-on-malformed behavior and the precise LZ4 output. That pins `lz4_flex` to a single version and freezes our error rules: a "bugfix" to `decode` is a hard fork. Biggest long-term cost and the least obvious. (Inherent to any in-protocol compression, not specific to this design.)

2. **Compute/DA asymmetry — a bounded decompression bomb.** A registered sequencer can post a few-KB compressed blob declaring `logical_len = MAX_LOGICAL_BLOB_LEN` (16 MiB); a full read forces the prover to decode up to 16 MiB (cycles) and hold it in memory (single frame ⇒ no streaming; peak ≈ 16 MiB per blob). Bounded by the cap and by gas (charged on logical length, `capabilities.rs:1046`, so the attacker pays roughly in proportion), and unregistered senders are gated at 1000 bytes — but decode is now on the prover's critical path, and cheap-to-post / expensive-to-verify is a real shape. (Also inherent to compression.)

3. **No partial-read optimization (latent).** A single frame can't be prefix-decoded, so *any* non-zero read decompresses the whole blob and authenticates all its shares. Moot today (reads are full-or-zero, Fact A), but it forecloses a future "peek a prefix cheaply" path without a format v2. See open point #2.

4. **Operational footgun → slashing.** Turning `compress_on_submit` on before every node/prover is upgraded means an old node reads an envelope as raw → borsh fails → an **honest sequencer is slashed**. The `off` default + "upgrade everyone first" discipline mitigates it, but a config mistake is punished severely.

5. **A non-obvious invariant.** Correct logical `total_len()` depends on construction having eager-advanced the 24-byte header into the accumulator at *every* construction site, plus an `OnceLock` interior-mutability cache to return `&[u8]` from a lazy decode. Reviewable, but more mechanism than "just decompress," and a place bugs can hide.

6. **Rides on top of the pre-existing witness bloat.** Envelope blobs still carry their (now compressed, hence smaller) shares in the witness; compression neither fixes nor worsens the unread-share bloat. Pruning is a separate follow-up.

## Accepted risks / out of scope
- Historical magic-collision reinterpretation: ~2⁻¹²⁸, documented.
- Old binaries can't decode envelope blobs: enabling `lz4` needs all nodes/provers on PR2+; `off` default makes rollout operator-controlled. (See disadvantage #4.)
- zstd reserved only (codec 2 → InvalidEnvelope/empty for current code; adding it later is a coordinated upgrade).
- **Witness unread-share bloat: pre-existing, unrelated to compression, not worsened. DECIDED: deferred to a separate follow-up** (Nikolai, 2026-06-15).
- Economics (bytes saved vs zk cycles): intentionally unmeasured; `off` default.
- `hash()` partial-authentication: pre-existing, unchanged.

## Open points
1. ~~Drop witness-pruning~~ — **RESOLVED: deferred to a follow-up.**
2. **Eager-advance the 24-byte header at construction** (v3) vs. a stored `logical_len` claim checked by the verifier. v3 picks eager-advance (no new field, no verifier change). See disadvantage #5.
3. **Single frame, no chunks** — accept that a future partial-read feature would need a v2 (chunked) format. See disadvantage #3.

## Design intent: preserve partial reads

Today's standard blob-storage STF path effectively reads blobs full-or-zero: size gates may skip a blob using `total_len()`, and accepted blobs are deserialized via `full_data()` on native / `verified_data()` in guest. That is a current usage pattern, not a limitation of the DA/STF model.

`BlobReaderTrait`, `CountedBufReader`, Celestia proof generation, and Celestia verification are intentionally built around partial consumption. The Celestia adapter should preserve that behavior even if the current standard STF does not use it, so a future STF/parser can stop after a malformed prefix of a large blob without forcing the prover/verifier to authenticate the whole blob.

Compression support should therefore avoid making envelope blobs inherently all-or-nothing unless that tradeoff is explicitly accepted as a separate format-version decision.
