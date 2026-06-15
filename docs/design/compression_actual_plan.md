# Celestia Blob Compression — Actual Plan & Progress (v5)

Status: ACTIVE — reworked from v4 after another security review (2026-06-15). Three
changes from v4: (1) the "full-or-zero" read fact is scoped to today's blob-storage
consumer, not to the DA adapter contract; (2) PR2 is **raw-envelope only** and proves the
chunked read/authentication model before any compression codec is added; (3) malformed or
insufficient envelope evidence must never become trusted empty/zero-filled logical bytes.
Scope: `crates/adapters/celestia` (self-contained; no public API changes). The existing
compressed-byte verifier checks remain the foundation, but PR2 adds envelope-specific
metadata/read-cursor authentication where those checks alone do not authenticate
STF-observable logical fields.
Supersedes: `celestia-compression-{claude,claude-2,codex,codex-2}.md` (kept for history).

## Decisions that frame this design (2026-06-15)

- **Chunked envelope from the start.** Independently-decodable fixed-size logical chunks,
  not a single frame. Rationale: preserve the partial-read capability `BlobReaderTrait`/
  `CountedBufReader` were built around (see "Design intent"), and bound decode work
  per-chunk. **Honest caveat:** Fact A says no current blob-storage caller does partial reads, so
  chunking buys no benefit *today*; it is forward-looking, and it enlarges the
  consensus-critical/audit surface (disadvantage #1). Single-frame would be cheaper and a
  smaller audit, at the cost of the DA adapter's partial-read property.
- **Prover is adversarial toward sequencers.** A malicious prover must not be able to
  slash an honest sequencer, nor attest an invalid transition as valid. This is what makes
  Fact C a must-fix rather than cosmetic.
- **No fabricated logical bytes.** Witness-restored data is prover-controlled until
  authenticated. PR2 must not introduce a trusted `Verified` semantic type, and malformed
  envelope evidence must not become a valid empty blob or zero-filled caller payload.
- **Envelope interpretation and emission are activation-gated.** `compress_on_submit` is
  not enough by itself. Before the protocol activation height/version, all blobs are
  interpreted as legacy raw even if they start with the magic prefix. After activation,
  emission may produce envelopes only when the local config allows it.

## Progress

| PR | Content | Status |
|----|---------|--------|
| PR1 | Accessor split (`compressed_*`/`logical_*`) + `TotalLenMismatch` security fix | ✅ **Committed** (`43fb15f92`) |
| PR2 | Raw chunked envelope read path: `envelope.rs` + chunk-aware authenticated reads on `BlobWithSender` (dark; nothing emits envelopes) | ⏳ Not started |
| PR3 | LZ4 codec + submit path + `compress_on_submit` + activation gate + metrics + e2e | ⏳ Not started |

**Dropped:** the earlier "witness-pruning via a `Native`/`Verified` enum + custom serde" PR. It was over-engineered, and the `Verified` label implied trust that does not exist — witness contents are attacker-controlled until authenticated. Witness-pruning is now **out of scope** (the pre-existing unread-share bloat is unrelated to compression and not worsened by it).

Process: each PR is implemented + verified in the working tree; **Nikolai reviews and commits himself**. The next PR starts only after the previous one is committed.

## Three facts that shape the design

### Fact A — current blob-storage reads are full-or-zero, but the DA adapter contract is partial-read capable

Today's standard `sov-blob-storage` consumer reads a DA blob in exactly one of two ways:
- **Full read** — `data_for_deserialization(blob)` returns `full_data()` (native) / `verified_data()` (guest), then borsh `try_from_slice` deserializes the whole slice in one call (`sov-blob-storage/src/capabilities.rs:1059`).
- **Zero data bytes read** — the blob is skipped by a `total_len()`-only gate *before* any payload byte is read: capacity limiter (`capabilities.rs:150`), emergency-registration size gate (`:230`, `MAX_EMERGENCY_REGISTRATION_BLOB_SIZE = 1000`), gas pre-charge (`:1046`), `batch_selector`.

There is **no current blob-storage path that reads a non-trivial prefix and abandons the
rest.** The completeness assert (`capabilities.rs:1063`, `verified_data().len() ==
total_len()`) fires only after a *failed* deserialization — i.e. after a full read.

This is a current consumer pattern, **not** a limitation of the DA adapter contract:
`BlobReaderTrait`, `CountedBufReader`, Celestia proof generation, and Celestia verification
are intentionally built around partial consumption. Existing Celestia tests already cover
no-read, single-byte, half-read, and full-read verification modes.

**Implication for chunking:** chunking adds *no value to today's blob-storage consumer*.
We adopt it to preserve the DA adapter's partial-read contract and to bound per-chunk
decode work. This is a deliberate, paid-for choice, not a Fact-A requirement.

### Fact B — the verifier trusts nothing from the witness

Trust comes only from the block header and the Celestia protocol, never from prover-serialized witness data:
1. `block_header` is the trusted public input. `validate_dah()` checks `dah.hash() == header.data_hash` → binds row roots to the header.
2. `proof.verify_range(row_root, shares.map(s.data()), namespace)` (nmt-rs) authenticates **share bytes**.
3. `sequence_length` / `signer` / `payload()` from the proven first share are trusted.
4. Witness fields are authenticated **against** those shares: the consumed-bytes accumulator is **byte-compared vs `share.payload()`** (`verifier/mod.rs:385-420`); `compressed_total_len() == sequence_length` (PR1, `:451-456`); `sender == recovered signer` (`:391-396`). Shares proven `= shares_needed_for_bytes_with_signer(accumulator.len()).max(1)` (`:355-369`), matched exactly → work scales with bytes consumed.

Because the accumulator is byte-compared against `share.payload()`, and shares hold the **compressed/on-DA** bytes, **the accumulator is compressed bytes**, authenticated as-is. Logical bytes cannot be tied to shares — they are derived from the authenticated accumulator. The witness must never carry trusted logical bytes. It may carry a logical read cursor or a decoded cache for implementation convenience, but PR2 must authenticate/recompute every STF-observable logical field against the authenticated compressed prefix before use. Proof generation uses `compressed_*` exclusively (`proofs.rs:175,188`).

### Fact C — the verifier+STF interlock that stops a truncating prover on failure paths

Today, on the standard deserialization-failure path, a malicious prover cannot slash an honest sequencer by truncating the witness: bytes are authenticated (it cannot corrupt them), and *under-reading* is caught by an interlock:
- the verifier pins `compressed_total_len() == sequence_length` (`verifier/mod.rs:451`) — note `total_len() = inner.remaining() + accumulator.len()`, so the prover cannot lie about the blob's full length; and
- the STF asserts `verified_data().len() == total_len()` (`capabilities.rs:1063`, *"…some data was not provided. The prover might be malicious"*).

A prover who authenticates only a 500-of-1000-byte prefix (legal at the compressed-prefix proof layer — the verifier proves only `shares_needed(500)` shares against the real prefix) trips the assert (`500 != 1000`) if deserialization fails → guest panic → no valid proof.

**Compression must not break this interlock or the size-gate paths that run before deserialization.** The v3 `decode` rule ("force output to exactly `logical_len`, zero-fill on any short/error") *does* break it: logical `total_len()` becomes a header value decoupled from the authenticated compressed length, and `verified_data()` becomes `decode(prefix)` zero-filled back to `logical_len`, so the assert degenerates to `logical_len == logical_len` (always true). A malicious prover could then authenticate a prefix → zero-fill → borsh fails on garbage → **honest sequencer slashed with a valid proof.** The corrected contract below restores the failure-path interlock and adds verifier/read-path checks for STF-observable metadata, so missing or malformed envelope evidence cannot silently turn into empty, skipped, or zero-filled logical bytes. The deserialize-success prefix/trailing-byte edge remains a pre-existing risk called out below and must be confirmed during PR2.

## Design

### Envelope format v1 (consensus-critical adapter constants, NOT config)

```
magic        [16] b"SOV_CELESTIA_CMP"   (ASCII, no 0x00 — share zero-padding can't fake it)
version      [1]  = 1
codec        [1]  0 = raw; 1 = LZ4 block (valid only once PR3 activates it);
                   2 reserved for zstd
flags        [2]  must be 0
logical_len  [4]  u32 LE, <= MAX_LOGICAL_BLOB_LEN
-- 24-byte fixed header. num_chunks = ceil(logical_len / CHUNK_LOGICAL_LEN) is derived
-- from logical_len, so the chunk table size is known after the header.
chunk_table  [4 * num_chunks]  u32 LE compressed length of each chunk
payload      [..]  num_chunks back-to-back chunks; chunk i is `chunk_table[i]` bytes.
                   codec=0: raw bytes whose chunk length equals the logical span.
                   codec=1: LZ4 block decoding is added only in PR3 after the PR2
                   authentication/read-cursor model is locked.
```
Constants: `MAX_LOGICAL_BLOB_LEN = 16 MiB` (decompression-bomb cap); `CHUNK_LOGICAL_LEN =
64 KiB` (fixed logical chunk size). Fixed logical chunk size keeps the logical→chunk map
arithmetic (`chunk = logical_offset / CHUNK_LOGICAL_LEN`) — only *compressed* chunk lengths
need storing, and the table for a 16 MiB blob is ≤256×4 B ≈ 1 KiB. The header +
chunk table are authenticated as normal consumed compressed bytes: they are included in
the accumulator and byte-compared against shares. This is bounded metadata overhead, not
free; large envelopes may require a few shares even for a logical zero-read.

### Mode detection — activation-gated and derived from authenticated bytes, never claimed

`mode = f(activation_status, authenticated first-share payload, accumulator_prefix, sequence_length)`, one shared pure function. The verifier must derive the same mode from activation status and authenticated share bytes before accepting STF-observable envelope fields:
- before activation → **LegacyRaw** regardless of magic bytes. This prevents historical or pre-upgrade magic-prefixed payloads from being reinterpreted.
- no 16-byte magic prefix in the authenticated first-share payload → **LegacyRaw** (today's behavior, byte-for-byte).
- magic + complete, valid PR2 header (version 1, codec 0, flags 0, `logical_len ≤ MAX`) + complete chunk table consistent with `logical_len` and `compressed_total_len` → **RawEnvelope**.
- magic + complete but invalid/oversized/unsupported header, inconsistent table, or unsupported codec → **MalformedEnvelopeAsRaw**. This is authenticated malformed DA content, but it is exposed under legacy raw semantics (`total_len() = compressed_total_len`, `verified_data() = compressed_verified_data()`), never as an empty blob and never as decoded logical bytes.
- magic present but the accumulator does not include the complete header/table needed to classify the blob, or does not include the compressed chunks needed for the claimed logical cursor → **InsufficientEnvelopeEvidence** → witness/proof error before STF execution. It must never be treated as malformed-authenticated content or as a valid empty blob.

Chunk-table validity uses checked arithmetic throughout: derive the table byte length only after `logical_len <= MAX`, cap each encoded length, reject offsets outside `sequence_length`, require `sum(encoded_lengths) == compressed_total_len - metadata_len`, and require PR2 `codec=0` chunk lengths to equal their logical chunk span exactly. In PR3, codec=1 becomes valid only after activation and only if its non-fabricating failure semantics are fully specified and tested.

### Deterministic, chunk-aware read mapping (no fabricated logical bytes)

`read_mapping(accumulator, compressed_total_len, logical_verified_len) -> ReadOutput`, identical native and guest. It may expose only bytes that are either authenticated raw DA bytes or deterministic logical bytes derived from authenticated envelope chunks. It must distinguish authenticated malformed content from insufficient witness evidence:

- **LegacyRaw / MalformedEnvelopeAsRaw:** no decoding. `verified_data()` is the compressed accumulator, returned as raw logical bytes under today's legacy semantics; `total_len()` is `compressed_total_len`. This preserves current gas/size/slash behavior for malformed authenticated DA content and avoids the unsafe "invalid envelope becomes empty" shortcut.
- **RawEnvelope with enough authenticated evidence:** `verified_data()` is exactly the first `logical_verified_len` bytes assembled from complete authenticated raw chunks. `total_len()` is the authenticated envelope `logical_len`.
- **RawEnvelope with only a prefix authenticated:** expose only complete chunks fully contained in the accumulator. A partial/truncated trailing chunk contributes **zero** logical bytes; it is never padded or zero-filled. `logical_verified_len` must be `<= available_decoded_len`; otherwise the verifier rejects `InsufficientEnvelopeEvidence` before the STF. For today's full-read path, real truncation yields `< logical_len` and the STF completeness assert (`capabilities.rs:1063`) fires on deserialization failure. For a legitimate future partial read, the reader returns exactly the requested logical prefix.

PR3 may add LZ4 chunk decoding only under the same rule: no unsupported/malformed/short/over-long compressed chunk may fabricate caller bytes. If this cannot be implemented while preserving monotonic partial-read behavior, LZ4 stays disabled and only codec=0 envelopes are valid.

The prover cannot cheat: the guest derives the same read mapping from the same authenticated accumulator the prover's native execution consumed, so any divergence yields a different state root than committed (invalid proof). The `MAX_LOGICAL_BLOB_LEN` and per-chunk caps live in this shared read-mapping code, applied identically native and guest.

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
- `verified_data()` (logical): LegacyRaw/MalformedEnvelopeAsRaw → `blob.accumulator()` (zero-overhead). RawEnvelope → assemble/cache exactly `logical_verified_len` logical bytes from authenticated chunks.
- `total_len()` (logical): LegacyRaw/MalformedEnvelopeAsRaw → `compressed_total_len()`. RawEnvelope → `logical_len` parsed from the authenticated header/table.
- `advance(n)` (native): Legacy → `blob.advance(n)`. Envelope → increment `logical_verified_len` by exactly `n` (clamped to `logical_total_len`) and advance the *compressed* accumulator to cover the chunks spanning that logical prefix (`chunk = logical_offset / CHUNK_LOGICAL_LEN`; consume those chunks' compressed bytes per the table). The only caller today is `full_data()` (n = the rest), which consumes the whole frame; the chunk machinery exists so a future partial reader can stop after a malformed prefix.
- The decode cache must be invalidated or keyed whenever either `compressed_verified_data().len()` or `logical_verified_len` changes. A bare `OnceLock<Vec<u8>>` is invalid because `verified_data()` may be called before a later `advance()`.
- **Construction** (`get_blobs_with_sender`, native, `types/mod.rs:200-234`): peek the first share for the magic; if it looks like an envelope, eagerly `advance` the **header + chunk table** bytes needed for classification so logical `total_len()` and the chunk map are available before any read (the size gates call `total_len()` first). These metadata bytes are authenticated like any other consumed bytes; expect bounded small proof overhead rather than zero extra shares.
- Manual `PartialEq` over `{blob, logical_verified_len, range, sender, hash}` (ignore the decoded cache).

### Verifier additions

Layer 1 stays exactly the existing compressed-byte verifier behavior: authenticate the
compressed accumulator against shares, check `compressed_total_len == sequence_length`
(PR1, `verifier/mod.rs:451`), and keep proof generation over `compressed_*`.

PR2 adds an envelope Layer 2 before handing blobs to the STF:
- derive mode from the authenticated first share and the authenticated compressed
  accumulator, not from witness claims;
- require complete authenticated header + chunk table before accepting any RawEnvelope
  `total_len()` or chunk map;
- validate the logical read cursor (`logical_verified_len <= decoded_available_len`) and
  reject `InsufficientEnvelopeEvidence`;
- treat authenticated malformed/unsupported envelope bytes as legacy raw bytes, not as
  empty, decoded, or zero-filled logical payload.

This keeps verifier errors reserved for witness/proof lies and deterministic decode results
reserved for authenticated DA content.

### Submit path & config (PR3)

- `CelestiaConfig.compress_on_submit: CompressOnSubmit` — `#[serde(rename_all="lowercase")] enum { #[default] Off, Lz4, Zstd }` (+ JsonSchema, following the `TxPriority` pattern in `config.rs`). Construction rejects `Zstd` ("reserved"). Envelope emission also requires a protocol activation height/version; the local flag alone is never sufficient to emit.
- `send_transaction` (batch namespace): after activation, `Lz4` → `encode` (split logical into `CHUNK_LOGICAL_LEN` chunks, compress each, build header + table + chunks; fall back to passthrough if the envelope is not strictly smaller). **Always** wrap magic-prefixed payloads in a codec=0 envelope even when compression is otherwise `Off` after activation (keeps legacy detection unambiguous). `send_proof` → magic-escape only after activation. `get_proofs_at_inner` → logical read mapping for transparency. Hash/commitment math untouched (`da_service/mod.rs:116`, commitment is over posted bytes).
- Metrics (logical vs posted bytes, saved basis points, outcome).

## PR breakdown

### PR1 — Accessor split + `total_len` auth ✅ committed (`43fb15f92`)

### PR2 — Raw chunked envelope read path (dark)
- New `crates/adapters/celestia/src/envelope.rs` (not cfg-gated; guest needs the shared read mapping): constants (`MAGIC`, `MAX_LOGICAL_BLOB_LEN`, `CHUNK_LOGICAL_LEN`), `Codec`, header + chunk-table parse, activation-aware `classify`, raw-envelope read mapping, `#[cfg(feature="native")] encode_raw`, `starts_with_magic`. Codec 0 is the only valid envelope codec in PR2 after activation. Codec 1/2 and malformed metadata classify as `MalformedEnvelopeAsRaw`, not empty.
- No compression dependency in PR2. This keeps the first security PR focused on authenticated metadata, logical cursor validation, and partial-read semantics.
- `types/mod.rs`: add `logical_verified_len` plus an invalidatable/keyed decoded cache + manual `PartialEq`; make `verified_data()`/`total_len()`/`advance()` chunk-aware; `get_blobs_with_sender` peeks the magic and eager-advances only the authenticated metadata bytes needed for classification.
- `verifier/mod.rs`: keep Layer 1 compressed checks, then add envelope Layer 2 for mode derivation, metadata completeness, chunk-table validity, logical cursor authentication, and malformed-as-raw behavior. `verifier/proofs.rs` remains over `compressed_*`.
- Tests: `envelope` unit (pre-activation magic-prefixed bytes stay LegacyRaw; parse matrix incl. magic-needs-16-bytes; activated raw-envelope round trips across sizes spanning multiple chunks; truncated trailing chunk ⇒ fewer logical bytes than `logical_len`, NOT zero-fill; `logical_len > MAX`, codec=1/2 before codec activation, inconsistent authenticated table ⇒ MalformedEnvelopeAsRaw; incomplete header/table/chunk evidence ⇒ InsufficientEnvelopeEvidence/reject; partial read at and across chunk boundaries returns the exact logical prefix); `types` (legacy unchanged; malformed envelope behaves like raw bytes; envelope `total_len()` pre-read via eager metadata; full read maps raw chunks; partial `advance(1)` exposes exactly one byte; cache invalidates after later `advance`; truncated read trips the completeness assert on failure paths); a checked-in raw-envelope fixture (`test_data/block_with_raw_envelope_blobs/`, generated once via docker) run through the verification matrix to prove raw-envelope blobs verify after activation, malformed authenticated content stays raw, and insufficient evidence is rejected. All existing tests pass untouched.
- Determinism gate: a test asserting native==guest read mapping over a fixed corpus.
- Witness compatibility: if `logical_verified_len` is serialized, PR2 is a witness-format change unless custom/defaulted serde preserves legacy decoding. Changelog notes read-path support + any witness compatibility impact + historical magic-collision caveat (~2⁻¹²⁸; such a v1 batch blob would have failed borsh anyway).

### PR3 — LZ4 codec + submit path + config + e2e
- Add workspace `lz4_flex` only in PR3, and only after a non-fabricating LZ4 failure rule is specified. The default safety rule is: unsupported/malformed/short/over-long chunk evidence must not produce zero-filled caller bytes. If preserving that rule with monotonic partial reads proves impractical, ship PR3 with codec=0 emission only and keep LZ4 disabled.
- `config.rs` enum + field + protocol activation height/version gate; `da_service/mod.rs` chunked encode-on-submit (+ magic-escape, fall-back-if-larger), `get_proofs_at` logical read mapping; metrics; documented `#compress_on_submit = "off"` in `examples/demo-rollup/configs/celestia_rollup_config.toml`; adapter README; changelog.
- Tests: config parse/default/roundtrip/schema; activation gate rejects pre-activation emission even if `compress_on_submit = "lz4"`; pre-activation reader treats magic-prefixed data as LegacyRaw; prepare-payload unit matrix (off-passthrough pre-activation, magic-escape post-activation, incompressible-fallback, compressible-shrinks, proof-never-compressed unless explicitly magic-escaped); malformed LZ4 never zero-fills or becomes empty; `get_proofs_at` parity; docker e2e (activated lz4 service: compressible + incompressible + magic-prefixed batches + a proof → read-back equals originals; extraction proof verifies for read/skip; posted < logical for the compressible one); mixed legacy+raw-envelope+lz4 namespace read by a PR2/PR3-aware service.

## Verification (each PR)
```bash
cargo fmt --all
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-celestia-adapter --features native
cargo check -p sov-celestia-adapter --no-default-features   # guest closure (read mapping must build)
```
Plus: the native==guest read-mapping determinism test, and the explicit Fact-C test (truncated
trailing chunk yields `< total_len()` logical bytes so the completeness assert fires).
PR2/PR3 each need one fixture-generation session against the dockerized Celestia devnet.

## Disadvantages & tradeoffs (honest assessment)

Ordered roughly by how much they should weigh on the decision.

1. **The read mapping becomes frozen, consensus-critical code — enlarged by chunking.** Once an envelope blob is posted, every node and prover must forever map it byte-identically — the exact chunk framing, cursor checks, malformed-as-raw behavior, and logical↔compressed map. PR3 adds LZ4 only if its failure semantics can be made equally deterministic without fabricating bytes. Biggest long-term cost. Mitigations: native==guest determinism CI; versioned format spec; add `lz4_flex` only once PR3's failure rule is locked.

2. **Compute/DA asymmetry once LZ4 is enabled — a bounded decompression bomb.** A registered sequencer can post a few-KB compressed blob declaring a large `logical_len`; a full read forces the prover to decode it. **Bounded three ways:** the 16 MiB per-blob cap, gas charged on logical `total_len()` (`capabilities.rs:1046`, so the attacker pays ~in proportion), and the block capacity limiter measured in logical bytes (`:150`, so a block can't be packed with many max bombs). Chunking improves peak memory and lets a future reader stop early. Residual: decode is on the prover's critical path. Accept only with caps + `off` default + activation-gated emission.

3. **DA adapter partial-read capability — delivered.** Chunking makes envelope blobs prefix-decodable: a future blob parser can authenticate and map only the chunks covering a prefix and stop after an early malformed region, without forcing the whole blob. Bought at the cost in #1. No current blob-storage consumer (Fact A).

4. **Operational footgun → slashing, fixed by activation-gated interpretation and emission.** Turning `compress_on_submit` on before every node/prover is upgraded means an old node reads an envelope as raw → borsh fails → an **honest sequencer is slashed**. PR3 therefore gates both envelope interpretation and emission on protocol activation height/version in addition to the local config flag. The `off` default remains useful operationally, but it is not the safety boundary.

5. **Non-obvious invariants (bug-hiding surface).** Correct logical `total_len()` depends on construction having eager-advanced enough header + chunk-table bytes into the accumulator at *every* construction site; plus an invalidatable/keyed cache to return `&[u8]` from lazy mapping; plus the logical cursor; plus the chunk-aware logical↔compressed map; plus the Fact-C prefix rule. All reviewable, but this is the place bugs hide. Covered by the explicit tests above; audit target.

6. **Rides on top of the pre-existing witness bloat.** Envelope blobs still carry their (now compressed, hence smaller) shares in the witness; compression neither fixes nor worsens the unread-share bloat. Pruning is a separate follow-up.

## Accepted risks / out of scope
- Historical magic-collision reinterpretation: ~2⁻¹²⁸, documented (outcome is slash either way).
- Old binaries can't decode envelope blobs: enabling envelope interpretation/emission needs all nodes/provers on PR2+. PR3's activation gate is the safety boundary; operators must upgrade before the activation height.
- zstd reserved only (codec 2 → MalformedEnvelopeAsRaw until a coordinated upgrade activates it).
- **Witness unread-share bloat: pre-existing, unrelated to compression, not worsened. DECIDED: deferred to a separate follow-up** (Nikolai, 2026-06-15).
- Borsh trailing-byte / valid-prefix edge on the deserialize-*success* branch (the assert at `capabilities.rs:1063` only runs on failure): pre-existing for legacy blobs, not introduced by compression. Worth a one-line confirmation during PR2, not a blocker.
- Economics (bytes saved vs zk cycles): intentionally unmeasured; `off` default.
- `hash()` partial-authentication: pre-existing, unchanged.

## Open points
1. ~~Drop witness-pruning~~ — **RESOLVED: deferred to a follow-up.**
2. **Eager-advance the header + chunk table at construction** vs. a stored claim checked by the verifier. Picks eager-advance plus verifier validation of the authenticated metadata; this is bounded metadata proof overhead, not zero-cost. See disadvantage #5.
3. ~~Single frame vs chunks~~ — **RESOLVED (2026-06-15): chunked**, to preserve partial reads. See "Decisions" and disadvantage #1 for the cost accepted.
4. ~~Activation gating~~ — **RESOLVED (2026-06-15): adopted.** PR3 envelope interpretation/emission requires protocol activation height/version; emission additionally requires the local config. See disadvantage #4.

## Design intent: preserve partial reads — RESOLVED by chunking

Today's standard blob-storage path effectively reads blobs full-or-zero (Fact A): size
gates may skip a blob using `total_len()`, and accepted blobs are deserialized via
`full_data()` / `verified_data()`. That is a current usage pattern, not a limitation of the
DA adapter contract — `BlobReaderTrait`, `CountedBufReader`, Celestia proof generation, and Celestia
verification are intentionally built around partial consumption.

The v3 single-frame envelope would have made envelope blobs inherently all-or-nothing,
foreclosing that capability without a format v2. **v5 resolves the tension by chunking from
the start:** legacy blobs keep partial-read behavior unchanged, and envelope blobs become
prefix-decodable at chunk granularity. The Fact-C read-mapping contract guarantees a partial read
never fabricates logical bytes (a not-fully-authenticated chunk yields nothing), so partial
reads are both *available* and *safe*. The accepted cost is the larger consensus-critical /
audit surface (disadvantage #1); the benefit has no blob-storage consumer yet (Fact A) and
is purely forward-looking.
