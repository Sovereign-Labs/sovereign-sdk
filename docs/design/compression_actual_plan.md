# Celestia Blob Compression - Actual Plan & Progress (v7)

Status: ACTIVE - v7, revised after another review on 2026-06-15.

v7 replaces the earlier raw-chunked PR2 with a much smaller plumbing-only PR2.
The first live envelope is now PR3, and it is a full-frame envelope rather than
a chunked partial-read format.

Scope remains `crates/adapters/celestia`. PR2 should not change live blob
behavior. PR3 changes Celestia adapter submission/read behavior for nodes that
run the new code and enable compression emission in their local runtime config.

## Decisions

- **Runtime config controls emission only.** `compress_on_submit` is a DA adapter
  runtime option. It must not affect guest verifier/STF read semantics, because
  local runtime config is not available in the proof. Once envelope-reading code
  is compiled in, interpretation is deterministic from posted bytes and adapter
  code. Operators must keep emission off until their network is upgraded.
- **No protocol activation height in this adapter plan.** The previous
  `ACTIVATION_HEIGHT` language was wrong for an SDK-level DA adapter. There is no
  universal Celestia height that is correct for all rollups.
- **Full-frame envelope v1.** Legacy blobs keep today's partial-read behavior.
  Envelope blobs are all-or-nothing: if a blob starts with the envelope magic,
  native extraction eagerly reads and classifies the whole posted frame before
  STF accessors are used. This avoids a logical cursor, avoids chunk-table
  consensus surface, and keeps `total_len()` stable even when decode fails.
- **No fabricated logical bytes.** Missing, malformed, short, or failed decode
  evidence must never become zero-filled or padded caller bytes.
- **Proof math stays physical.** Celestia inclusion proof generation and
  verification continue to use `compressed_*` accessors because shares commit to
  posted DA bytes, not decoded logical bytes.
- **Witness pruning stays out of scope.** Existing witness bloat for unread
  shares is independent of compression and is not solved in this series.

## Current Facts

The standard blob-storage consumer is effectively full-or-zero today:

- Size/capacity gates call `total_len()` before reading payload bytes.
- Accepted blobs are deserialized from `full_data()` in native mode and
  `verified_data()` in guest mode.
- On deserialization failure, the STF asserts that all claimed data was provided
  before slashing the sequencer.

The Celestia verifier already authenticates the DA-physical accumulator against
shares and checks `compressed_total_len() == sequence_length`. This remains the
core trust boundary. The witness may record what was read, but the witness does
not make bytes or lengths trusted.

The old PR2 plan had two important problems:

- A skipped `Option<EnvelopeMode>` cannot be filled by the guest verifier through
  the current `DaVerifier::verify_relevant_tx_list(&RelevantBlobs, ...)` API. A
  skipped interior cache is needed if the public API stays unchanged.
- Chunked compressed partial reads require a logical cursor. Without that cursor,
  a native `advance(1)` that authenticates a whole chunk cannot be reproduced
  precisely in the guest.

## Envelope Format v1

The format is full-frame, not chunked:

```text
magic        [16] b"SOV_CELESTIA_CMP"
version      [1]  = 1
codec        [1]  0 = raw escape; 1 = LZ4 block; 2 reserved for zstd
flags        [2]  must be 0
logical_len  [4]  u32 LE, <= MAX_LOGICAL_BLOB_LEN
payload      [..] codec-specific full-frame payload
```

Constants:

- `MAX_LOGICAL_BLOB_LEN`: maximum decoded payload length.
- `MAX_COMPRESSION_RATIO`: maximum allowed `logical_len / posted_payload_len`
  before decoding.

Codec rules:

- `codec = 0`: payload is raw logical bytes and must have length
  `logical_len`. This exists mainly to escape payloads that naturally start with
  the magic prefix.
- `codec = 1`: payload is one LZ4-compressed block that must decode exactly to
  `logical_len`.
- Unknown versions, nonzero flags, unsupported codecs, cap violations,
  malformed headers, length mismatch, and LZ4 decode failure classify as
  malformed-as-raw after the posted bytes are authenticated.

There is intentionally no chunk table and no partial compressed read support in
v1.

## Blob State And Read Semantics

`BlobWithSender` should grow skipped derived state, not serialized witness
claims:

```rust
#[serde(skip)]
envelope_state: OnceLock<EnvelopeState>
```

The exact type name is not important, but the semantics are:

- The cache is interior-mutable so the verifier/accessors can initialize it
  while holding only `&BlobWithSender`.
- The cache is ignored by serialization and by `PartialEq`.
- It stores only data derived from the authenticated compressed accumulator:
  legacy/malformed/envelope mode plus decoded bytes for a valid envelope.
- Accessors must not trust a serialized mode claim.

Read behavior:

- **Legacy raw:** unchanged. `verified_data()` returns the compressed
  accumulator, `total_len()` is `compressed_total_len()`, and native `advance(n)`
  consumes `n` physical bytes.
- **Valid envelope:** native extraction has already consumed the whole posted
  frame. `verified_data()` returns the decoded/full raw logical payload,
  `total_len()` returns `logical_len`, and `advance(_)` is effectively a no-op
  after construction.
- **Malformed-as-raw:** native extraction has already consumed the whole posted
  frame. `verified_data()` returns the posted bytes, and `total_len()` is
  `compressed_total_len()`. This makes bad authenticated content slashable under
  existing legacy semantics.

This deliberately gives up partial-read savings for envelope blobs. That is the
cost paid to avoid a serialized logical cursor and error-returning blob
accessors.

## Verifier Semantics

Layer 1 stays unchanged:

- Authenticate `compressed_verified_data()` against Celestia shares.
- Check `compressed_total_len()` against the first share's `sequence_length`.
- Keep proof generation over `compressed_*` accessors.

Envelope Layer 2 is added when live envelope interpretation is introduced:

- If the compressed accumulator does not start with the magic prefix, keep legacy
  raw behavior.
- If it starts with the magic prefix, require the accumulator to contain the full
  posted frame before handing the blob to the STF.
- Recompute envelope state from authenticated posted bytes only.
- Valid envelope state exposes decoded logical bytes.
- Malformed envelope state exposes authenticated posted bytes as raw legacy data.
- Insufficient evidence is a verifier/proof error, not an empty blob and not
  zero-filled data.

Because envelope blobs are eagerly full-frame, `total_len()` is known and stable
before blob-storage size gates and gas precharge run. Decode failure cannot flip
the blob from logical-length accounting to raw-length accounting after those
gates have already executed.

## PR Breakdown

### PR1 - Accessor split plus `total_len` authentication

Committed (`43fb15f92`). This remains the correct foundation.

### PR2 - Plumbing only

Goal: prepare the adapter for envelope-aware code without changing live blob
behavior.

- Add `envelope.rs` with constants, magic detection, fixed-header parsing,
  classification helpers, cap checks, and test-only fixtures/helpers as needed.
- Add skipped `OnceLock<EnvelopeState>` or equivalent interior cache to
  `BlobWithSender`, with manual `PartialEq` ignoring the cache.
- Keep `verified_data()`, `total_len()`, `advance()`, verifier behavior, proof
  generation, and submission behavior unchanged for real blobs.
- Add tests proving skipped derived state is reconstructed after serde round
  trips and that native/guest helper classification is deterministic.
- No LZ4 dependency, no runtime config, no live envelope interpretation, no
  raw-envelope fixture, and no activation/chain-param changes.

PR2 is allowed to introduce unused helper code if tests cover it, because the
purpose is to de-risk the skipped cache and deterministic parsing before the
live compression PR.

### PR3 - Full-frame LZ4 envelope and emission config

Goal: introduce the first live envelope behavior.

- Add `CelestiaConfig.compress_on_submit = off | lz4`, default `off`.
- Add `lz4_flex` and full-frame LZ4 encode/decode helpers.
- On submission, if `compress_on_submit = lz4`, compress batch blobs and wrap
  only when the posted envelope is strictly smaller than raw.
- If a raw payload starts with the magic prefix after PR3, wrap it in `codec = 0`
  so future readers do not confuse it with malformed envelope bytes.
- Keep proof blobs raw unless explicitly decided otherwise; if proof blobs are
  subject to magic escaping, `get_proofs_at` must return logical proof bytes.
- In native extraction, magic-prefixed blobs are eagerly read and classified over
  the full posted frame. Valid envelopes cache decoded logical bytes; malformed
  envelopes cache raw posted bytes.
- In guest verification, the same classification/decoding is recomputed from the
  authenticated compressed accumulator before the STF uses the blob.
- Add metrics for raw bytes, posted bytes, compression outcome, and bytes saved.

PR3 is the first PR where old binaries may misinterpret newly emitted envelopes
as raw bytes. The operational rule is simple: keep `compress_on_submit = off`
until all relevant nodes/provers have upgraded.

## Verification

Run for every Rust PR:

```bash
cargo fmt --all
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-celestia-adapter --features native
cargo check -p sov-celestia-adapter --no-default-features
```

Additional PR2 tests:

- Serde round trip drops skipped envelope cache and reconstructs it
  deterministically.
- Parser/classifier matrix for legacy, valid header, malformed header, cap
  violations, unsupported codec, and incomplete evidence.
- Existing Celestia verification tests pass unchanged.

Additional PR3 tests:

- Legacy raw blobs are unchanged.
- Valid LZ4 envelope round trips.
- Incompressible payloads fall back to raw.
- Magic-prefixed raw payloads are escaped with `codec = 0`.
- Malformed envelope bytes behave as authenticated raw bytes and are slashable.
- Insufficient evidence rejects before STF execution.
- Truncated witness data still trips the existing completeness protection.
- Docker e2e covers raw, compressed, incompressible fallback, magic escape, and
  proof retrieval behavior.

## Accepted Risks And Tradeoffs

- No read-side activation gate means upgraded code interprets magic-prefixed
  historical blobs as envelopes. The magic is 128 bits and chosen so standard
  Borsh batch/proof payloads starting with it would already be malformed in
  practice. This is an accepted collision risk.
- Runtime config cannot make read semantics safe for mixed old/new networks. It
  only prevents this node from emitting envelopes. Coordinated rollout remains
  an operational requirement.
- Envelope blobs lose partial-read savings in v1. This keeps the first
  compression implementation auditable and avoids a serialized logical cursor.
- Eager full-frame classification means malformed magic-prefixed blobs can force
  full posted-byte authentication before being slashed or discarded. Caps and the
  DA block size bound keep this finite.
- Compression economics are intentionally left to metrics. Default emission is
  off.
