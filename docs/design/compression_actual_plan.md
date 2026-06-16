# Celestia Blob Compression - Actual Plan & Progress (v10)

Status: ACTIVE - v10, revised on 2026-06-16.

**v8 reversed v7's full-frame mistake (compression MUST be streaming / chunked); v9–v10
incorporate two codex review rounds:** mode is header-only and immutable, and the
corrupt-chunk completeness halt is fixed by a small consensus-critical **Prerequisite
PR-A** (`BlobReaderTrait` + `sov-blob-storage`) that lands before PR3. Round 2 hardened
PR-A: the decode-error check runs **before accepting any Borsh `Ok`** (a corrupt
envelope can decode to a short prefix that is itself valid Borsh), and
`logical_decode_failed()` requires a **canonical** decode (exact length, exact physical
consumption, no trailing/extra/malformed chunks).
The detailed design lives in `docs/design/celestia-compression-pr3.md`; the four
investigation docs (`celestia-compression-claude{,-2}.md`,
`celestia-compression-codex{,-2}.md`) are the authoritative source for the chunked
approach.

Scope remains `crates/adapters/celestia`. PR2 did not change live blob behavior.
PR3 changes Celestia adapter submission/read behavior for nodes that run the new
code and enable compression emission in their local runtime config.

## ⚠️ Streaming / chunked compression is mandatory (corrects v7)

The verifier today proves only the shares the STF actually read: **verifier and
guest work scale with the bytes the rollup consumed, not with the full blob size.**
This is a hard security property (no DoS where reading K bytes forces the verifier
to authenticate/decompress the whole blob).

A **full-frame** (single-frame, all-or-nothing) compressed envelope **violates**
this: a monolithic LZ4/zstd frame must be decoded in its entirety to produce any
output byte, so exposing one logical byte would force authentication + decompression
of every share of the blob. v7 accepted that regression ("Envelope blobs lose
partial-read savings"). **That is not acceptable.**

The fix is **independently-decodable chunks**: the compressed payload is a sequence
of small chunks (share-aligned in v1), each decodable on its own. To read the first
N logical bytes the verifier authenticates and decodes only the chunks covering
them. v7's objection that "chunked reads need a serialized logical cursor the guest
cannot reproduce" is wrong: the per-chunk framing (`chunk_logical_len`,
`chunk_encoded_len`) is self-delimiting and is part of the authenticated compressed
bytes — it *is* the cursor. The guest recomputes
`logical = decode(complete chunks in the authenticated compressed prefix)`
bit-for-bit, with no separate serialization. Native reads advance by whole chunks,
so `compressed_verified_data` always ends on a chunk boundary.

## Decisions

- **Streaming / chunked envelope (v1).** Compression is chunked and
  prefix-decodable. Legacy blobs keep today's behavior. See "Envelope Format v1".
- **Runtime config controls emission only.** `compress_on_submit` is a DA adapter
  runtime option. It must not affect guest verifier/STF read semantics, because
  local runtime config is not available in the proof. Once envelope-reading code is
  compiled in, interpretation is deterministic from posted bytes and adapter code.
  Operators must keep emission off until their network is upgraded.
- **No protocol activation height in this adapter plan.** There is no universal
  Celestia height correct for all rollups.
- **No fabricated logical bytes.** Missing, malformed, short, or failed-decode
  evidence must never become zero-filled or padded caller bytes. A header-valid
  envelope whose authenticated chunks don't fully decode reports
  `logical_decode_failed()` and is **slashed** (Prerequisite PR-A) — NOT routed
  through the completeness assert, which would panic-halt (see PR-A below).
- **Proof math stays physical.** Inclusion proof generation and verification use
  the `compressed_*` accessors because shares commit to posted DA bytes, not decoded
  logical bytes.
- **Witness pruning stays out of scope.** Existing witness bloat for unread shares
  is independent of compression and is not solved in this series.

## Current Facts

The standard blob-storage consumer is effectively full-or-zero today:

- Size/capacity gates call `total_len()` before reading payload bytes.
- Accepted blobs are deserialized from `full_data()` in native mode and
  `verified_data()` in guest mode.
- On deserialization failure, the STF asserts that all claimed data was provided
  (`verified_data().len() == total_len()`) before slashing the sequencer. This
  assert is the fail-closed backstop for a prover-withheld / incomplete witness (PR-A
  makes it run unconditionally, before Borsh — see Prerequisite PR-A).

The Celestia verifier already authenticates the DA-physical accumulator against
shares and checks `compressed_total_len() == sequence_length`. This remains the
core trust boundary. The witness may record what was read, but the witness does not
make bytes or lengths trusted.

On v7's two arguments for abandoning chunking (both wrong):

- A skipped interior cache (e.g. `OnceLock<EnvelopeState>`) is indeed needed so the
  guest verifier can fill derived state through `&BlobWithSender`. That argues for
  the interior cache (PR2 has it) — **not** for full-frame. The chunked design uses
  the same cache.
- "Chunked partial reads require a logical cursor that the guest cannot reproduce"
  is false: chunk framing is self-delimiting and authenticated, so a native
  `advance` that consumes whole chunks is reproduced precisely in the guest by
  decoding the same authenticated compressed prefix.

## Envelope Format v1

Chunked, prefix-decodable. All little-endian.

```text
fixed header (24 bytes):
  magic             [16]  b"SOV_CELESTIA_CMP"
  version           [1]   = 1
  codec             [1]   0 = raw chunk, 1 = LZ4 block
  flags             [2]   = 0
  logical_len       [4]   u32, total decoded length, <= MAX_LOGICAL_BLOB_LEN
then a sequence of chunks (the payload):
  chunk_logical_len [2]   u16, <= MAX_LOGICAL_CHUNK_LEN
  chunk_encoded_len [2]   u16, <= MAX_ENCODED_CHUNK_LEN
  chunk_payload     [chunk_encoded_len]
```

Constants (deterministic verifier rules, not config):

- `MAX_LOGICAL_BLOB_LEN`: maximum total decoded payload length (64 MiB).
- `MAX_LOGICAL_CHUNK_LEN`: maximum decoded length of a single chunk (≈ one
  continuation-share payload, ~482 B — share-aligned). This is the per-byte DoS
  bound and the partial-read granularity.
- `MAX_ENCODED_CHUNK_LEN`: maximum posted length of a single chunk (LZ4 max-output
  of `MAX_LOGICAL_CHUNK_LEN`).

Codec rules:

- `codec = 0` (raw chunk): each chunk is verbatim, `chunk_encoded_len ==
  chunk_logical_len`. Used to escape payloads that naturally start with the magic.
- `codec = 1` (LZ4 block): each chunk is an independent LZ4 block decodable without
  later chunks. Decode uses the framing-supplied known output size.
- The verifier enforces, **before allocating** each chunk's output:
  `1 <= chunk_logical_len <= MAX_LOGICAL_CHUNK_LEN` (**zero-logical chunks rejected**,
  else a reader could scan unbounded empty chunks for one logical byte),
  `1 <= chunk_encoded_len <= MAX_ENCODED_CHUNK_LEN`, and the running
  `sum(chunk_logical_len) <= logical_len`; a complete decode must satisfy
  `sum == logical_len`.
- **Header-level** failures (unknown version, nonzero flags, unsupported codec,
  `logical_len` over cap, short/malformed fixed header) → **malformed-as-raw** after
  the posted bytes are authenticated. Mode is decided from the header alone and is
  immutable.
- **Post-header** failures of a valid-header envelope (corrupt chunk, decompression
  failure, `sum < logical_len`) do NOT flip the mode — they set
  `logical_decode_failed()` → **slash** (Prerequisite PR-A). Layer 2 never returns
  `Err` for envelope content.

There is **no per-blob `max_logical_chunk_len` header field**: the fixed global cap
already bounds every chunk. The 24-byte header is byte-identical to PR2's; only the
payload is reinterpreted from a single frame to a chunk stream. No v1 envelope was
ever emitted (emission defaults off), so v1 := chunked from PR3 on.

## Blob State And Read Semantics

`BlobWithSender` keeps the compressed reader and grows skipped derived state, not
serialized witness claims:

```rust
#[serde(skip)]
envelope_state: OnceLock<EnvelopeState>   // decode-once cache; Envelope carries decoded logical bytes
// plus, native only:
#[cfg(feature = "native")] #[serde(skip)]
native_decode: ...                        // incremental decode buffer, mutated via &mut advance
```

Semantics:

- The cache is interior-mutable so the verifier/accessors can initialize it while
  holding only `&BlobWithSender`; it is ignored by serialization and by `PartialEq`;
  it stores only data derived from the authenticated compressed accumulator. The
  native incremental buffer is also serde-skipped and excluded from `PartialEq`.
- Accessors must not trust a serialized mode claim.

Read behavior:

- **Legacy raw:** unchanged. `verified_data()` returns the compressed accumulator,
  `total_len()` is `compressed_total_len()`, native `advance(n)` consumes `n`
  physical bytes.
- **Valid envelope (streaming):** `verified_data()` returns the decoded logical
  bytes for the chunks read so far; `total_len()` returns the authenticated header
  `logical_len`; native `advance(n)` decodes forward by **whole chunks** until ≥ `n`
  more logical bytes are available, pulling only the compressed chunks it needs. The
  guest recomputes the logical prefix from the authenticated compressed prefix.
- **Malformed-as-raw (header-level):** `verified_data()` returns the posted bytes and
  `total_len()` is `compressed_total_len()`, so bad authenticated content is slashable
  under legacy semantics.
- **Header-valid but undecodable** (corrupt chunk / too few chunks): stays an Envelope
  (`total_len() = logical_len`, no mode flip); `verified_data()` is the short decoded
  prefix; `logical_decode_failed()` is true → blob-storage **slashes** (Prerequisite
  PR-A), not a panic.

This **preserves** partial-read savings for compressed blobs (the whole point) and
avoids a serialized logical cursor (the chunk framing is the cursor).

## Verifier Semantics

Layer 1 stays unchanged:

- Authenticate `compressed_verified_data()` against Celestia shares.
- Check `compressed_total_len()` against the first share's `sequence_length`.
- Keep proof generation over `compressed_*` accessors. Shares proven scale with the
  compressed bytes consumed (chunk-granular) — the DoS property is preserved.

Envelope Layer 2 (chunked) is added:

- If the authenticated compressed prefix does not start with the magic, keep legacy
  raw behavior.
- If it starts with the magic and `compressed_verified_data().len() >=
  ENVELOPE_HEADER_LEN`, parse + validate the (authenticated) header and decode the
  chunks within the compressed prefix, enforcing per-chunk caps before allocation.
  The decoded result is the logical payload the STF reads.
- Mode is **header-only and immutable.** A magic prefix with a short/invalid header is
  **malformed-as-raw** (authenticated posted bytes exposed as legacy raw data). A
  valid header stays an Envelope for life; a malformed/undecodable chunk does NOT flip
  the mode — it sets `logical_decode_failed()` and is **slashed** via Prerequisite
  PR-A. Layer 2 never returns `Err`; only Layer-1 (share / signer / sequence-length)
  failures are verifier errors. This keeps a malicious poster from halting the chain
  with one bad blob — it is slashed instead.
- Insufficient header evidence (magic present, fewer than `ENVELOPE_HEADER_LEN`
  authenticated bytes) falls back to malformed-as-raw, so `total_len()` never
  exposes an unauthenticated `logical_len`.

Because the header is eagerly read at construction and proven by Layer 1,
`total_len()` (= authenticated `logical_len`) is known and stable before
blob-storage size gates and gas precharge run. `total_len()` is never clamped to the
decodable length, so the STF completeness assert remains the fail-closed backstop
for a prover-withheld / incomplete witness. (A corrupt but fully-present envelope is
caught earlier and **slashed** via Prerequisite PR-A's `logical_decode_failed()`, not
asserted; PR-A also moves that assert to run unconditionally, before Borsh.)

## PR Breakdown

### PR1 - Accessor split plus `total_len` authentication

Committed (`43fb15f92`). The correct foundation: `compressed_*`/`logical_*` accessor
split; proof generation uses the `compressed_*` accessors.

### PR2 - Plumbing only

Committed. Adds `envelope.rs` (magic detection, fixed-header parsing,
classification, caps) and the skipped `OnceLock<EnvelopeState>` cache, without
changing live blob behavior. NOTE: PR2 landed a *full-frame* parser; PR3 reworks the
payload interpretation to a chunk stream (the 24-byte header is unchanged).

### Prerequisite PR-A - authenticated DA decode-error signal (lands before PR3)

A small consensus-critical change so an authenticated-but-undecodable envelope slashes
instead of panic-halting. A valid-header envelope whose authenticated chunks don't
decode canonically leaves `verified_data().len() < total_len()`, which
`capabilities.rs:1063`'s completeness assert turns into a **panic** (permissionless
chain halt). Add `BlobReaderTrait::logical_decode_failed(&self) -> bool` (default
`false` → legacy unchanged), true iff the fully-present authenticated bytes don't form
a **clean, exact, complete** decode (exact `logical_len`, exact physical consumption,
no trailing/extra/malformed chunks). Restructure `deserialize_or_try_slash_sender` (the
single accept chokepoint for batches + `Vec<u8>` proofs) to slash on
`logical_decode_failed()` **before accepting any Borsh `Ok`** (a corrupt envelope can
decode to a short prefix that is itself valid Borsh), and move the completeness assert
to run **unconditionally before Borsh** (R3: a prover can withhold trailing bytes and
expose a prefix that is itself valid Borsh — an Err-arm-only assert would miss it; this
also closes the same pre-existing legacy gap). Touches `crates/rollup-interface`
+ `sov-blob-storage`, reviewed in isolation; PR3 depends on it. Full spec in
`celestia-compression-pr3.md`.

### PR3 - Chunked streaming LZ4 envelope and emission config

Goal: introduce the first live envelope behavior, **streaming/chunked**. Full design
in `docs/design/celestia-compression-pr3.md`. Summary:

- Add `CelestiaConfig.compress_on_submit = off | lz4` (default `off`) and
  `compression_chunk_size` (default share-aligned).
- Add `lz4_flex` (non-optional, `default-features = false, features =
  ["safe-encode", "safe-decode"]` — bare `default-features = false` selects the
  *unsafe* codec) and chunked encode/decode helpers (block API, framing-supplied
  known size). Implement the celestia side of PR-A's `logical_decode_failed()`.
- On submission with `compress_on_submit = lz4`, compress batch blobs into a chunked
  envelope and post it only when strictly smaller than raw; escape magic-prefixed
  raw batch payloads with `codec = 0`; proofs post verbatim.
- Native reads advance by whole chunks; guest verification recomputes the logical
  prefix from the authenticated compressed prefix (Layer 2).
- `get_proofs_at` decodes envelopes.
- Add metrics for logical bytes, posted bytes, mode, bytes saved, ratio (basis
  points), and chunk count.

PR3 is the first PR where old binaries may misinterpret newly emitted envelopes as
raw bytes. Operational rule: keep `compress_on_submit = off` until all relevant
nodes/provers have upgraded.

## Verification

Run for every Rust PR:

```bash
cargo fmt --all
SKIP_GUEST_BUILD=1 cargo nextest run -p sov-celestia-adapter --features native
cargo check -p sov-celestia-adapter --no-default-features
```

Additional PR3 tests (see `celestia-compression-pr3.md` for the full matrix):

- Legacy raw blobs unchanged.
- Compressed envelope partial-read matrix (no-read / one-chunk / half / full).
- Header-authentication invariant (forged short accumulator + huge claimed
  `logical_len` → recomputed malformed).
- Truncated-final-chunk property test (strict prefix, never panic).
- Corrupt interior chunk / too-few chunks → sequencer slashed via PR-A
  (`logical_decode_failed`), not a panic; identically native/guest.
- Malformed-magic header → slashable raw (chain does not halt).
- Negative verifier matrix (caps, truncation, decode failure, mutated bytes).
- Docker e2e: raw, compressed, incompressible fallback, magic escape, proof retrieval.

## Accepted Risks And Tradeoffs

- No read-side activation gate means upgraded code interprets magic-prefixed
  historical blobs as envelopes. The 128-bit magic is chosen so a standard Borsh
  batch/proof payload that is also a *valid* envelope is negligible; a magic-prefixed
  *invalid* frame is malformed-as-raw, i.e. identical bytes to legacy. Accepted
  collision risk.
- Runtime config cannot make read semantics safe for mixed old/new networks; it only
  prevents this node from emitting envelopes. Coordinated rollout remains an
  operational requirement.
- Chunked compression **preserves** partial-read savings and the verifier DoS bound
  (work scales with compressed bytes consumed, chunk-granular). Share-aligned chunks
  add small framing overhead (~0.8% on a large batch); accepted for the tightest
  granularity.
- Decompression cost is bounded per-chunk (caps enforced before allocation) and by
  `MAX_LOGICAL_BLOB_LEN`. Compression economics are left to metrics; default emission
  is off.
