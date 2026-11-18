# Celestia Fibre adapter: handoff

State of `sov-celestia-fibre-adapter` (PRs #3095 + #3126, branch `nikolai/fibre-zk-verifier`,
productization on `nikolai/fibre-adaptper-productization`). The gaps were found by comparing with
#3136 and with `sov-celestia-adapter` (the "classic" adapter).

## Done

- **Downloads use the validator set of the block's height.** `download_fibre_blobs` passes
  `DownloadOptions { height: Some(height), .. }`. With the default (`None`), lumina uses the
  validator set at the chain head (`celestia-fibre` 1.2.0, `client/download.rs`), which can
  differ from the set that stored the blob when the node is catching up.
- **Celestia node calls have the classic adapter's timeouts and retries.** Config params with the
  same names and defaults as `CelestiaConfig` (`request_timeout_secs`, `api_request_timeout_secs`,
  `backoff_*`); node calls run through `CelestiaFibreDaService::call_with_retries`. The defaults
  are duplicated in `crates/adapters/celestia-fibre/src/config.rs` and must be kept in sync.
- **Header height check.** `get_block_header_at` rejects a header whose height differs from the
  requested one and retries it as transient, like the classic adapter.
- **Configurable per-validator timeout (`fibre_request_timeout_secs`, default 30).** Lumina
  already bounds every validator request, connection setup included, with a client-side timeout
  of 30 seconds (`celestia-grpc-macros` 1.2.0, `lib.rs`, `lumina_utils::time::timeout` around
  the call), so an unresponsive validator never stalls a download. The `FibreClient` is built
  like `FibreClient::from_grpc_client`, but with `GrpcValidatorConnector::with_request_timeout`
  (`da_service.rs`, `fibre_client`) to make that value configurable. It needs a direct
  `celestia-fibre` dependency, because `celestia-client` doesn't re-export the connector types.
- **Upload retries.** `upload_and_prepare` is retried with `fibre_backoff` (about a minute) when it
  fails. A failed upload returns no payment promise, so a retry can't pay twice. Validators that
  stored shards of a failed attempt hold its promise, though, and could charge it after it times
  out (celestia-app `x/fibre`, `MsgPaymentPromiseTimeout`).
- **Submission no longer pays twice on a timeout.** `submit_message` (broadcast + confirm) was
  retried as a whole, so a confirmation timeout re-signed and re-broadcast the same payment
  promise. The chain settles a promise only once ("payment promise has already been processed",
  celestia-app `x/fibre/keeper/keeper.go`), so every retry failed and the blob sender then
  uploaded and paid again. Now `broadcast_message` is retried, and `confirm_pay_for_fibre` polls
  `tx_status` with retries, re-broadcasts the same signed bytes on eviction (like
  `celestia-grpc`), never re-signs, and accepts the "already processed" rejection of a duplicate.
- **Config parity:** RPC and gRPC fallback endpoints, `SOV_CELESTIA_*` environment-variable
  defaults, startup validation of URLs, and a separate `fibre_grpc_url`/`fibre_grpc_auth_token`
  for Fibre's validator-set and host queries. `verify_on_fetch_mode` is intentionally absent:
  `get_block_at` always verifies the namespace data against the header, because it builds the
  namespace proof from it.
- **Metrics** (`metrics.rs`): every attempt of header, head, namespace-data, broadcast, tx-status,
  Fibre download and Fibre upload calls, with duration, outcome and, for Fibre, blob size.
- **Docs and packaging:** CHANGELOG entry (`#PR_NUMBER` placeholder), crate README, committed
  example config `examples/demo-rollup/configs/celestia_fibre_rollup_config.toml` (with a test that
  it parses), `publish = true`, and listed in `packages_to_publish.yml` after
  `sov-celestia-adapter`.
- **Share-count formula is shared:**
  `sov_celestia_adapter::shares::shares_needed_for_bytes_with_signer` is now public and used by
  `namespace::sequence_shares`.

## Remaining

- **Detecting an already-settled promise is best effort.** The chain has
  `Query/IsPaymentProcessed`, but `celestia-proto` 1.2.0 doesn't compile `fibre/v1/query.proto`,
  so there's no client for it. A duplicate rejected in the mempool is reported with ABCI code 1,
  which `celestia-types` 1.2.0 can't convert (`ErrorCode::try_from`), so `tx_status` fails and the
  error text is lost (found by reading source, not observed). This only happens when a broadcast
  attempt times out after reaching the mempool. A lumina change exposing `IsPaymentProcessed`
  would close it.
- **Mocha genesis** stays local (`examples/test-data/genesis/demo/mocha-fibre`): it holds a
  personal signer address and DA height. The example config explains how to derive one.
- **Typed verifier error: not done, on purpose.** `CelestiaFibreVerifier` returns `anyhow::Error`
  while the classic verifier returns `ValidationError`. `DaVerifier::Error` only requires `Debug`,
  and nothing outside the classic crate matches on `ValidationError` variants, so a typed error
  would mean rewriting every check in `namespace.rs`, `commitment.rs` and `spec.rs` with no
  consumer.
- **No service-level test harness.** The submission and confirmation paths are only covered by a
  manual mocha run (2026-11-16, chain `mocha-5`): demo-rollup with
  `celestia_fibre_rollup_config.toml`, secrets from `SOV_CELESTIA_*`, a fresh genesis near the tip.
  15 blobs were submitted and confirmed with a height, the sequencer recovered and became ready,
  and the node read its own batches back. Duplicate-promise and eviction handling were not
  exercised.
