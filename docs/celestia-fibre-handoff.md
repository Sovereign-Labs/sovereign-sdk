# Celestia Fibre adapter: handoff

State of `sov-celestia-fibre-adapter` (PRs #3095 + #3126, branch `nikolai/fibre-zk-verifier`)
and the work deferred to follow-up PRs. The gaps were found by comparing with #3136 and with
`sov-celestia-adapter` (the "classic" adapter).

## Done in this branch

- **Downloads use the validator set of the block's height.** `download_fibre_blobs` passes
  `DownloadOptions { height: Some(height), .. }`. With the default (`None`), lumina uses the
  validator set at the chain head (`celestia-fibre` 1.2.0, `client/download.rs`), which can
  differ from the set that stored the blob when the node is catching up.
- **Celestia node calls have the classic adapter's timeouts and retries.**
  - Config params, with the same names and defaults as `CelestiaConfig`: `request_timeout_secs`
    (20), `api_request_timeout_secs` (5), `backoff_min_delay_ms` (100), `backoff_max_delay_ms`
    (12000), `backoff_max_times` (60), `backoff_factor` (2.0). The defaults are duplicated in
    `crates/adapters/celestia-fibre/src/config.rs` and must be kept in sync.
  - `api_request_timeout_secs` is set on both the RPC client and the gRPC client.
  - Header by height, network head, namespace data and the `MsgPayForFibre` submission
    (`submit_message`) run through `CelestiaFibreDaService::call_with_retries`. Each attempt is
    bounded by `request_timeout_secs`, and every failure is retried as transient, like the
    classic adapter.

## Deferred

### Correctness / robustness

- **Header height check.** The classic adapter rejects a header whose height differs from the
  requested one ("Received wrong height", `crates/adapters/celestia/src/da_service/mod.rs`).
  `CelestiaFibreDaService::get_block_header_at` trusts the RPC response.
- **Traffic to Fibre validators** is not covered by the change above:
  - `FibreClient::download` has a backoff (`download_backoff`, about one minute in total) but no
    per-attempt timeout. Lumina's validator connections have no request timeout by default
    (`transport/grpc_validator_client.rs`). It is not verified whether a single unresponsive
    validator can stall a download.
  - `FibreClient::upload_and_prepare` in `submit` has neither a timeout nor retries.
- **Resubmission after a timeout.** As in the classic adapter, a `submit_message` attempt that
  times out is retried, although the transaction may already have been included. Check how the
  chain treats a second `MsgPayForFibre` with the same payment promise, and how that error
  surfaces to the sequencer.

### Operational parity with the classic adapter

- **Config:** no RPC or gRPC fallback endpoints, no environment-variable defaults
  (`SOV_CELESTIA_RPC_URL`, ...), no `verify_on_fetch_mode`, no separate Fibre gRPC endpoint.
- **Metrics:** none. The classic adapter reports metrics for header, namespace-data and
  submission calls; #3136 also reports Fibre download, encode and submit.

### Docs and packaging

- No CHANGELOG entry.
- No crate README.
- No committed example config: `.gitignore` excludes
  `examples/demo-rollup/configs/mocha_fibre_config.toml`, and the mocha genesis is local only.
- The crate has `publish = false` and is not in `packages_to_publish.yml`. Change both if it
  should be published.

### Code reuse

- `namespace::sequence_shares` copies the share-count formula of the classic adapter's
  `shares_needed_for_bytes_with_signer`, which is crate-private there.
- `CelestiaFibreVerifier` returns `anyhow::Error`, while the classic verifier returns the typed
  `ValidationError`.
